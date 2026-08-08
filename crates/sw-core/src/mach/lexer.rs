//! Tokenizer for the boolean hardware expression syntax.

use crate::error::{Error, Result};

/// A lexical token of the boolean hardware syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// An attribute name or value, e.g. `CPUBOARD` or `IP22`.
    Word(String),
    /// `&&`
    And,
    /// `||`
    Or,
    /// `!`
    Not,
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `(`
    LParen,
    /// `)`
    RParen,
}

/// Splits a boolean hardware expression into tokens.
///
/// # Errors
///
/// Returns [`Error::MachSyntax`] on stray operator characters.
pub fn lex(input: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '&' if chars.get(i + 1) == Some(&'&') => {
                tokens.push(Token::And);
                i += 2;
            }
            '|' if chars.get(i + 1) == Some(&'|') => {
                tokens.push(Token::Or);
                i += 2;
            }
            '!' if chars.get(i + 1) == Some(&'=') => {
                tokens.push(Token::Ne);
                i += 2;
            }
            '!' => {
                tokens.push(Token::Not);
                i += 1;
            }
            '=' => {
                tokens.push(Token::Eq);
                i += 1;
            }
            '<' if chars.get(i + 1) == Some(&'=') => {
                tokens.push(Token::Le);
                i += 2;
            }
            '<' => {
                tokens.push(Token::Lt);
                i += 1;
            }
            '>' if chars.get(i + 1) == Some(&'=') => {
                tokens.push(Token::Ge);
                i += 2;
            }
            '>' => {
                tokens.push(Token::Gt);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            _ if is_word_char(c) => {
                let start = i;
                while i < chars.len() && is_word_char(chars[i]) {
                    i += 1;
                }
                tokens.push(Token::Word(chars[start..i].iter().collect()));
            }
            _ => {
                return Err(Error::MachSyntax {
                    raw: input.to_string(),
                    message: format!("unexpected character {c:?}"),
                });
            }
        }
    }

    Ok(tokens)
}

fn is_word_char(c: char) -> bool {
    !c.is_whitespace() && !"&|!<>=()".contains(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_boolean_expression() {
        let tokens = lex("CPUBOARD=IP22 && (GFXBOARD=EXPRESS || GFXBOARD!=NEWPRESS)").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Word("CPUBOARD".into()),
                Token::Eq,
                Token::Word("IP22".into()),
                Token::And,
                Token::LParen,
                Token::Word("GFXBOARD".into()),
                Token::Eq,
                Token::Word("EXPRESS".into()),
                Token::Or,
                Token::Word("GFXBOARD".into()),
                Token::Ne,
                Token::Word("NEWPRESS".into()),
                Token::RParen,
            ]
        );
    }

    #[test]
    fn rejects_stray_characters() {
        assert!(lex("CPUBOARD=IP22 & GFXBOARD=X").is_err());
    }
}
