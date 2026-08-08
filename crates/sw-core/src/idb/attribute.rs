//! IDB entry attributes.
//!
//! Attributes appear at the end of each IDB line as a free-form list, for
//! example:
//!
//! ```text
//! sum(61435) size(601) f(940005084) config(suggest) cmpsize(465)
//! ```
//!
//! They are numerous, may repeat, and their order can matter, so the
//! attribute list — not a fixed set of struct fields — is the source of
//! truth. Attributes this library does not understand are preserved
//! verbatim as [`IdbAttribute::Unknown`].
use crate::diagnostic::Diagnostic;
use crate::mach::HardwareExpr;

/// One IDB attribute.
#[derive(Debug, Clone, PartialEq)]
pub enum IdbAttribute {
    /// `sum(n)` — checksum of the installed file.
    Checksum(u64),
    /// `size(n)` — uncompressed size in bytes.
    Size(u64),
    /// `cmpsize(n)` — size as stored in the image archive; zero means the
    /// file is stored uncompressed.
    CompressedSize(u64),
    /// `config(mode)` — configuration file handling.
    Config(ConfigMode),
    /// `nohist` — do not record installation history.
    NoHistory,
    /// `delhist` — delete installation history.
    DeleteHistory,
    /// `dev(major minor)` — device numbers for special files.
    Device {
        /// Major device number.
        major: u32,
        /// Minor device number.
        minor: u32,
    },
    /// `symval(target)` — symbolic link target.
    SymlinkValue(String),
    /// `mach(expr)` — hardware applicability expression.
    Mach(HardwareExpr),
    /// A `mach(expr)` whose expression could not be parsed.
    ///
    /// Kept separate from [`IdbAttribute::Unknown`] so that selection can
    /// never mistake an unparseable hardware restriction for a mach-less
    /// entry (which would make it a common fallback — a fail-open).
    MachUnparsed {
        /// Raw attribute argument text.
        raw: String,
        /// Why parsing failed.
        error: String,
    },
    /// `exitop(cmd)` — command run after installation completes.
    ExitOp(String),
    /// `preop(cmd)` — command run before installing the file.
    PreOp(String),
    /// `postop(cmd)` — command run after installing the file.
    PostOp(String),
    /// `removeop(cmd)` — command run when the file is removed.
    RemoveOp(String),
    /// `shadow` — shadow file semantics.
    Shadow,
    /// `nostrip` — do not strip the binary.
    NoStrip,
    /// `stripdso` — strip a shared object.
    StripDso,
    /// `norqs` — do not run `rqs` on the binary.
    NoRqs,
    /// Any other attribute, preserved verbatim.
    Unknown {
        /// Attribute name.
        name: String,
        /// Whitespace-separated argument tokens inside the parentheses.
        args: Vec<String>,
        /// Raw attribute text, e.g. `f(940005084)`.
        raw: String,
    },
}

/// Modes of the `config(...)` attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigMode {
    /// `config(suggest)`
    Suggest,
    /// `config(noupdate)`
    NoUpdate,
    /// `config(update)`
    Update,
    /// Any other mode, preserved verbatim.
    Unknown(String),
}

/// One raw attribute token: `name` plus optional parenthesized argument.
pub(crate) struct AttributeToken {
    /// Attribute name, e.g. `cmpsize`.
    pub(crate) name: String,
    /// Argument text inside the parentheses, if any.
    pub(crate) args: Option<String>,
    /// Raw token text.
    pub(crate) raw: String,
}

impl AttributeToken {
    /// Whether the token carries a parenthesized argument list.
    pub(crate) fn has_args(&self) -> bool {
        self.args.is_some()
    }
}

/// Splits an attribute region into tokens.
///
/// The tokenizer is parenthesis- and quote-aware, so tokens like
/// `mach(CPUBOARD=IP19 CPUBOARD=IP21)` and `exitop("rm -rf ...")` stay
/// intact.
pub(crate) fn tokenize(
    input: &str,
    origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<AttributeToken> {
    let mut tokens = AttributeTokens {
        chars: input.chars().collect(),
        pos: 0,
        origin,
        diagnostics,
    };
    let mut out = Vec::new();
    for token in &mut tokens {
        out.push(token);
    }
    out
}

/// Classifies one token into an attribute.
///
/// Unparseable pieces are kept as [`IdbAttribute::Unknown`] whenever
/// possible; a diagnostic is emitted for pieces that had to be dropped.
pub(crate) fn classify(
    token: AttributeToken,
    origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> IdbAttribute {
    classify_token(token, origin, diagnostics)
}

/// Splits an attribute region into tokens, respecting double quotes so
/// that e.g. `exitop("rm -rf $rbase/foo")` stays one token.
struct AttributeTokens<'a> {
    chars: Vec<char>,
    pos: usize,
    origin: &'a str,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl Iterator for AttributeTokens<'_> {
    type Item = AttributeToken;

    fn next(&mut self) -> Option<AttributeToken> {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
        if self.pos >= self.chars.len() {
            return None;
        }

        let start = self.pos;
        while self.pos < self.chars.len()
            && !self.chars[self.pos].is_whitespace()
            && self.chars[self.pos] != '('
        {
            self.pos += 1;
        }
        let name_end = self.pos;

        let mut args = None;
        if self.pos < self.chars.len() && self.chars[self.pos] == '(' {
            let args_start = self.pos + 1;
            let mut depth = 0usize;
            let mut in_quotes = false;
            let mut closed = false;
            while self.pos < self.chars.len() {
                let c = self.chars[self.pos];
                match c {
                    '"' => in_quotes = !in_quotes,
                    '(' if !in_quotes => depth += 1,
                    ')' if !in_quotes => {
                        depth -= 1;
                        if depth == 0 {
                            closed = true;
                            break;
                        }
                    }
                    _ => {}
                }
                self.pos += 1;
            }
            if !closed {
                self.diagnostics.push(Diagnostic::warning(
                    "unterminated attribute argument list",
                    Some(self.origin.to_string()),
                ));
            }
            args = Some(self.chars[args_start..self.pos].iter().collect());
            if closed {
                self.pos += 1;
            }
        }

        let name: String = self.chars[start..name_end].iter().collect();
        let raw_end = self.pos;
        Some(AttributeToken {
            name,
            args,
            raw: self.chars[start..raw_end].iter().collect(),
        })
    }
}

fn classify_token(
    token: AttributeToken,
    origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> IdbAttribute {
    let unknown = |token: &AttributeToken| IdbAttribute::Unknown {
        name: token.name.clone(),
        args: token
            .args
            .as_deref()
            .map(|a| a.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
        raw: token.raw.clone(),
    };

    let numeric_arg = |token: &AttributeToken, index: usize| -> Option<u64> {
        token
            .args
            .as_deref()?
            .split_whitespace()
            .nth(index)?
            .parse()
            .ok()
    };

    match (token.name.as_str(), token.args.as_deref()) {
        ("sum", Some(_)) => numeric_arg(&token, 0)
            .map(IdbAttribute::Checksum)
            .unwrap_or_else(|| unknown(&token)),
        ("size", Some(_)) => numeric_arg(&token, 0)
            .map(IdbAttribute::Size)
            .unwrap_or_else(|| unknown(&token)),
        ("cmpsize", Some(_)) => numeric_arg(&token, 0)
            .map(IdbAttribute::CompressedSize)
            .unwrap_or_else(|| unknown(&token)),
        ("config", Some(args)) => IdbAttribute::Config(match args.trim() {
            "suggest" => ConfigMode::Suggest,
            "noupdate" => ConfigMode::NoUpdate,
            "update" => ConfigMode::Update,
            other => ConfigMode::Unknown(other.to_string()),
        }),
        ("nohist", None) => IdbAttribute::NoHistory,
        ("delhist", None) => IdbAttribute::DeleteHistory,
        ("dev", Some(_)) => match (numeric_arg(&token, 0), numeric_arg(&token, 1)) {
            (Some(major), Some(minor)) => IdbAttribute::Device {
                major: major as u32,
                minor: minor as u32,
            },
            _ => unknown(&token),
        },
        ("symval", Some(args)) => IdbAttribute::SymlinkValue(unquote(args)),
        ("mach", Some(args)) => match HardwareExpr::parse(args) {
            Ok(expr) => IdbAttribute::Mach(expr),
            Err(error) => {
                diagnostics.push(Diagnostic::warning(
                    format!("malformed mach attribute: {error}"),
                    Some(origin.to_string()),
                ));
                IdbAttribute::MachUnparsed {
                    raw: args.to_string(),
                    error: error.to_string(),
                }
            }
        },
        ("exitop", Some(args)) => IdbAttribute::ExitOp(unquote(args)),
        ("preop", Some(args)) => IdbAttribute::PreOp(unquote(args)),
        ("postop", Some(args)) => IdbAttribute::PostOp(unquote(args)),
        ("removeop", Some(args)) => IdbAttribute::RemoveOp(unquote(args)),
        ("shadow", None) => IdbAttribute::Shadow,
        ("nostrip", None) => IdbAttribute::NoStrip,
        ("stripdso", None) => IdbAttribute::StripDso,
        ("norqs", None) => IdbAttribute::NoRqs,
        _ => unknown(&token),
    }
}

/// Removes one layer of surrounding double quotes, if present.
fn unquote(args: &str) -> String {
    let trimmed = args.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_attrs(input: &str) -> (Vec<IdbAttribute>, Vec<Diagnostic>) {
        let mut diagnostics = Vec::new();
        let attributes = tokenize(input, "test.idb:1", &mut diagnostics)
            .into_iter()
            .map(|token| classify(token, "test.idb:1", &mut diagnostics))
            .collect();
        (attributes, diagnostics)
    }

    #[test]
    fn parses_common_attributes() {
        let (attributes, diagnostics) =
            parse_attrs("sum(61435) size(601) f(940005084) config(suggest) cmpsize(465)");
        assert!(diagnostics.is_empty());
        assert_eq!(
            attributes,
            vec![
                IdbAttribute::Checksum(61435),
                IdbAttribute::Size(601),
                IdbAttribute::Unknown {
                    name: "f".into(),
                    args: vec!["940005084".into()],
                    raw: "f(940005084)".into(),
                },
                IdbAttribute::Config(ConfigMode::Suggest),
                IdbAttribute::CompressedSize(465),
            ]
        );
    }

    #[test]
    fn parses_flags_and_devices() {
        let (attributes, _) =
            parse_attrs("nohist delhist dev(10 213) shadow nostrip stripdso norqs");
        assert_eq!(
            attributes,
            vec![
                IdbAttribute::NoHistory,
                IdbAttribute::DeleteHistory,
                IdbAttribute::Device {
                    major: 10,
                    minor: 213
                },
                IdbAttribute::Shadow,
                IdbAttribute::NoStrip,
                IdbAttribute::StripDso,
                IdbAttribute::NoRqs,
            ]
        );
    }

    #[test]
    fn keeps_quoted_operations_intact() {
        let (attributes, diagnostics) = parse_attrs(
            "exitop(\"rm -fr $rbase/etc/xutmp $rbase/bin/rsh\") postop($rbase/.bin.mv.sh)",
        );
        assert!(diagnostics.is_empty());
        assert_eq!(
            attributes,
            vec![
                IdbAttribute::ExitOp("rm -fr $rbase/etc/xutmp $rbase/bin/rsh".into()),
                IdbAttribute::PostOp("$rbase/.bin.mv.sh".into()),
            ]
        );
    }

    #[test]
    fn parses_mach_expressions() {
        let (attributes, diagnostics) = parse_attrs("mach(CPUBOARD=IP19 CPUBOARD=IP21)");
        assert!(diagnostics.is_empty());
        let [IdbAttribute::Mach(expr)] = attributes.as_slice() else {
            panic!("expected single mach attribute, got {attributes:?}");
        };
        assert_eq!(expr.raw, "CPUBOARD=IP19 CPUBOARD=IP21");
    }

    #[test]
    fn parses_symlink_values() {
        let (attributes, _) = parse_attrs("symval(../lib/libc.so.1)");
        assert_eq!(
            attributes,
            vec![IdbAttribute::SymlinkValue("../lib/libc.so.1".into())]
        );
    }
}
