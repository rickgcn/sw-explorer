//! Non-fatal problems collected while reading a distribution.

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Something looks off, but processing could continue losslessly.
    Warning,
    /// Part of the data had to be skipped or left incomplete.
    Error,
}

/// A non-fatal problem encountered while opening or parsing a distribution.
///
/// Fatal problems are reported as [`crate::error::Error`]; diagnostics cover
/// everything the library could recover from, so that callers can show them
/// to the user instead of silently "fixing" things.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity of the problem.
    pub severity: Severity,
    /// Human-readable description.
    pub message: String,
    /// Optional origin, e.g. `eoe.idb:18422`.
    pub origin: Option<String>,
}

impl Diagnostic {
    /// Creates a warning diagnostic.
    pub fn warning(message: impl Into<String>, origin: Option<String>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            message: message.into(),
            origin,
        }
    }

    /// Creates an error diagnostic.
    pub fn error(message: impl Into<String>, origin: Option<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            message: message.into(),
            origin,
        }
    }
}
