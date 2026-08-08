//! Parser producing [`HardwareExpr`] ASTs from raw expression text.

use super::lexer::{self, Token};
use super::{CompareOp, HardwareExpr, HardwareSyntax, MachAttribute, MachExpr};
use crate::error::{Error, Result};

impl HardwareExpr {
    /// Parses a raw hardware expression, auto-detecting the syntax.
    ///
    /// Text containing any of `&&`, `||`, `!`, `<`, `>` or parentheses is
    /// treated as the boolean syntax; anything else as the legacy list
    /// syntax.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MachSyntax`] if the expression is malformed.
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        let looks_boolean = ["&&", "||", "!", "<", ">", "(", ")"]
            .iter()
            .any(|marker| trimmed.contains(marker));

        let (syntax, expr) = if looks_boolean {
            (HardwareSyntax::Boolean, parse_boolean(trimmed)?)
        } else {
            (HardwareSyntax::LegacyList, parse_legacy_list(trimmed)?)
        };

        Ok(HardwareExpr {
            raw: raw.to_string(),
            syntax,
            expr,
        })
    }
}

/// Parses the legacy list syntax.
///
/// Same-attribute comparisons are OR-ed, different attributes are AND-ed:
/// `CPUBOARD=IP12 GFXBOARD=LIGHT GFXBOARD=EXPRESS` becomes
/// `CPUBOARD=IP12 AND (GFXBOARD=LIGHT OR GFXBOARD=EXPRESS)`.
///
/// A bare token without `=` is shorthand for a `CPUBOARD` value:
/// `IP22 IP26` becomes `CPUBOARD=IP22 OR CPUBOARD=IP26`.
fn parse_legacy_list(raw: &str) -> Result<MachExpr> {
    // Attribute order of first appearance, so the resulting tree is stable.
    let mut groups: Vec<(MachAttribute, Vec<MachExpr>)> = Vec::new();

    for token in raw.split_whitespace() {
        let (attribute, value) = match token.split_once('=') {
            Some(("", _)) => {
                return Err(Error::MachSyntax {
                    raw: raw.to_string(),
                    message: format!("legacy mach token {token:?} has an empty attribute"),
                });
            }
            Some((attribute, value)) => (MachAttribute::from_name(attribute), value),
            // Bare value: the attribute defaults to CPUBOARD.
            None => (MachAttribute::CpuBoard, token),
        };
        let compare = MachExpr::Compare {
            attribute: attribute.clone(),
            op: CompareOp::Eq,
            value: value.to_string(),
        };
        match groups.iter_mut().find(|(a, _)| *a == attribute) {
            Some((_, values)) => values.push(compare),
            None => groups.push((attribute, vec![compare])),
        }
    }

    if groups.is_empty() {
        return Err(Error::MachSyntax {
            raw: raw.to_string(),
            message: "empty mach expression".to_string(),
        });
    }

    let mut per_attribute: Vec<MachExpr> = groups
        .into_iter()
        .map(|(_, mut values)| {
            if values.len() == 1 {
                values.pop().expect("length checked")
            } else {
                MachExpr::Or(values)
            }
        })
        .collect();

    if per_attribute.len() == 1 {
        Ok(per_attribute.pop().expect("length checked"))
    } else {
        Ok(MachExpr::And(per_attribute))
    }
}

/// Parses the boolean syntax with a recursive descent parser.
///
/// Precedence, lowest to highest: `||`, `&&`, `!`, comparison / parens.
fn parse_boolean(raw: &str) -> Result<MachExpr> {
    let tokens = lexer::lex(raw)?;
    let mut cursor = Cursor {
        raw,
        tokens: &tokens,
        pos: 0,
    };
    let expr = cursor.parse_or()?;
    if cursor.pos != tokens.len() {
        return Err(cursor.error("trailing tokens"));
    }
    Ok(expr)
}

struct Cursor<'a> {
    raw: &'a str,
    tokens: &'a [Token],
    pos: usize,
}

impl Cursor<'_> {
    fn error(&self, message: &str) -> Error {
        Error::MachSyntax {
            raw: self.raw.to_string(),
            message: message.to_string(),
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn parse_or(&mut self) -> Result<MachExpr> {
        let mut operands = vec![self.parse_and()?];
        while self.peek() == Some(&Token::Or) {
            self.advance();
            operands.push(self.parse_and()?);
        }
        if operands.len() == 1 {
            Ok(operands.pop().expect("length checked"))
        } else {
            Ok(MachExpr::Or(operands))
        }
    }

    fn parse_and(&mut self) -> Result<MachExpr> {
        let mut operands = vec![self.parse_unary()?];
        while self.peek() == Some(&Token::And) {
            self.advance();
            operands.push(self.parse_unary()?);
        }
        if operands.len() == 1 {
            Ok(operands.pop().expect("length checked"))
        } else {
            Ok(MachExpr::And(operands))
        }
    }

    fn parse_unary(&mut self) -> Result<MachExpr> {
        if self.peek() == Some(&Token::Not) {
            self.advance();
            Ok(MachExpr::Not(Box::new(self.parse_unary()?)))
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<MachExpr> {
        match self.peek() {
            Some(Token::LParen) => {
                self.advance();
                let expr = self.parse_or()?;
                if self.peek() != Some(&Token::RParen) {
                    return Err(self.error("missing closing parenthesis"));
                }
                self.advance();
                Ok(expr)
            }
            Some(Token::Word(_)) => self.parse_comparison(),
            _ => Err(self.error("expected attribute comparison or parenthesized expression")),
        }
    }

    fn parse_comparison(&mut self) -> Result<MachExpr> {
        let Some(Token::Word(attribute)) = self.peek() else {
            return Err(self.error("expected attribute name"));
        };
        let attribute = attribute.clone();
        self.advance();

        let op = match self.peek() {
            Some(Token::Eq) => CompareOp::Eq,
            Some(Token::Ne) => CompareOp::Ne,
            Some(Token::Lt) => CompareOp::Lt,
            Some(Token::Le) => CompareOp::Le,
            Some(Token::Gt) => CompareOp::Gt,
            Some(Token::Ge) => CompareOp::Ge,
            // Bare value: the attribute defaults to CPUBOARD.
            _ => {
                return Ok(MachExpr::Compare {
                    attribute: MachAttribute::CpuBoard,
                    op: CompareOp::Eq,
                    value: attribute,
                });
            }
        };
        self.advance();

        let Some(Token::Word(value)) = self.peek() else {
            return Err(self.error("expected comparison value"));
        };
        let value = value.clone();
        self.advance();

        Ok(MachExpr::Compare {
            attribute: MachAttribute::from_name(&attribute),
            op,
            value,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_single_token() {
        let expr = HardwareExpr::parse("CPUBOARD=IP19").unwrap();
        assert_eq!(expr.syntax, HardwareSyntax::LegacyList);
        assert_eq!(
            expr.expr,
            MachExpr::Compare {
                attribute: MachAttribute::CpuBoard,
                op: CompareOp::Eq,
                value: "IP19".into(),
            }
        );
    }

    #[test]
    fn legacy_same_attribute_is_or() {
        let expr = HardwareExpr::parse("CPUBOARD=IP19 CPUBOARD=IP21 CPUBOARD=IP25").unwrap();
        let MachExpr::Or(values) = expr.expr else {
            panic!("expected Or, got {:?}", expr.expr);
        };
        assert_eq!(values.len(), 3);
    }

    #[test]
    fn legacy_different_attributes_are_and() {
        let expr = HardwareExpr::parse("CPUBOARD=IP12 GFXBOARD=LIGHT GFXBOARD=EXPRESS").unwrap();
        let MachExpr::And(groups) = expr.expr else {
            panic!("expected And, got {:?}", expr.expr);
        };
        assert_eq!(groups.len(), 2);
        assert!(matches!(groups[0], MachExpr::Compare { .. }));
        assert!(matches!(groups[1], MachExpr::Or(_)));
    }

    #[test]
    fn boolean_expression() {
        let expr = HardwareExpr::parse("CPUBOARD=IP22 && (GFXBOARD=EXPRESS || GFXBOARD=NEWPRESS)")
            .unwrap();
        assert_eq!(expr.syntax, HardwareSyntax::Boolean);
        let MachExpr::And(operands) = expr.expr else {
            panic!("expected And, got {:?}", expr.expr);
        };
        assert_eq!(operands.len(), 2);
        assert!(matches!(operands[1], MachExpr::Or(_)));
    }

    #[test]
    fn boolean_negation_and_inequality() {
        let expr = HardwareExpr::parse("MODE=64bit && CPUBOARD!=IP26").unwrap();
        assert!(matches!(expr.expr, MachExpr::And(_)));
        let expr = HardwareExpr::parse("!(CPUBOARD=IP26)").unwrap();
        assert!(matches!(expr.expr, MachExpr::Not(_)));
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(HardwareExpr::parse("").is_err());
        assert!(HardwareExpr::parse("=IP22").is_err());
        assert!(HardwareExpr::parse("CPUBOARD=IP22 &&").is_err());
        assert!(HardwareExpr::parse("(CPUBOARD=IP22").is_err());
    }

    #[test]
    fn bare_value_is_cpuboard_shorthand() {
        let expr = HardwareExpr::parse("IP22").unwrap();
        assert_eq!(expr.syntax, HardwareSyntax::LegacyList);
        assert_eq!(
            expr.expr,
            MachExpr::Compare {
                attribute: MachAttribute::CpuBoard,
                op: CompareOp::Eq,
                value: "IP22".into(),
            }
        );

        let expr = HardwareExpr::parse("IP22 IP26").unwrap();
        let MachExpr::Or(values) = expr.expr else {
            panic!("expected Or, got {:?}", expr.expr);
        };
        assert_eq!(values.len(), 2);
        assert!(values.iter().all(|value| matches!(
            value,
            MachExpr::Compare {
                attribute: MachAttribute::CpuBoard,
                ..
            }
        )));
    }

    #[test]
    fn boolean_bare_value_is_cpuboard_shorthand() {
        let expr = HardwareExpr::parse("IP22 || IP26").unwrap();
        assert_eq!(expr.syntax, HardwareSyntax::Boolean);
        let MachExpr::Or(values) = expr.expr else {
            panic!("expected Or, got {:?}", expr.expr);
        };
        assert_eq!(
            values,
            vec![
                MachExpr::Compare {
                    attribute: MachAttribute::CpuBoard,
                    op: CompareOp::Eq,
                    value: "IP22".into(),
                },
                MachExpr::Compare {
                    attribute: MachAttribute::CpuBoard,
                    op: CompareOp::Eq,
                    value: "IP26".into(),
                },
            ]
        );

        let expr = HardwareExpr::parse("IP22 && GFXBOARD=EXPRESS").unwrap();
        let MachExpr::And(operands) = expr.expr else {
            panic!("expected And, got {:?}", expr.expr);
        };
        assert_eq!(
            operands[0],
            MachExpr::Compare {
                attribute: MachAttribute::CpuBoard,
                op: CompareOp::Eq,
                value: "IP22".into(),
            }
        );
        assert_eq!(
            operands[1],
            MachExpr::Compare {
                attribute: MachAttribute::GfxBoard,
                op: CompareOp::Eq,
                value: "EXPRESS".into(),
            }
        );
    }
}
