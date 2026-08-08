//! Structured error type for all fallible `sw-core` operations.

use std::path::PathBuf;

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by `sw-core`.
///
/// Non-fatal problems (a single malformed IDB line, an undecoded descriptor
/// body, ...) are reported through [`crate::diagnostic::Diagnostic`] instead
/// of aborting the whole operation.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An underlying I/O operation failed.
    #[error("I/O error on {path}: {source}")]
    Io {
        /// Path of the file being accessed.
        path: PathBuf,
        /// Original I/O error.
        source: std::io::Error,
    },

    /// The given path is not a readable SGI distribution directory.
    #[error("not a distribution directory: {path}")]
    NotADistribution {
        /// Path that failed the check.
        path: PathBuf,
    },

    /// A product with the requested name does not exist in the distribution.
    #[error("product not found: {name}")]
    ProductNotFound {
        /// Requested product name.
        name: String,
    },

    /// An IDB line could not be parsed at all.
    #[error("IDB syntax error in {path} line {line}: {message}")]
    IdbSyntax {
        /// IDB file containing the bad line.
        path: PathBuf,
        /// 1-based line number.
        line: usize,
        /// Human-readable description.
        message: String,
    },

    /// A `mach` hardware expression is malformed.
    #[error("mach expression syntax error in {raw:?}: {message}")]
    MachSyntax {
        /// The raw expression text.
        raw: String,
        /// Human-readable description.
        message: String,
    },

    /// A product descriptor file is malformed or unreadable.
    #[error("descriptor error in {path}: {message}")]
    Descriptor {
        /// Descriptor file path.
        path: PathBuf,
        /// Human-readable description.
        message: String,
    },

    /// An image archive header is malformed.
    ///
    /// A valid archive starts with a 13-byte header: the `im001V…` magic
    /// terminated by a NUL byte.
    #[error("invalid image archive header in {path}: {message}")]
    ImageFormat {
        /// Archive file path.
        path: PathBuf,
        /// Human-readable description.
        message: String,
    },

    /// An entry has no payload locator, so its bytes cannot be read.
    #[error("entry has no payload: {entry}")]
    PayloadNotFound {
        /// Entry path.
        entry: String,
    },

    /// The size of a payload as stored is unknown, so its bytes cannot be
    /// delimited in the image archive.
    #[error("stored payload size is unknown: {entry}")]
    PayloadSizeUnknown {
        /// Entry path.
        entry: String,
    },

    /// The payload record could not be found in the image archive, even after
    /// a resynchronization scan.
    #[error("payload record not found for {entry}")]
    ResyncFailed {
        /// Entry path.
        entry: String,
    },

    /// A resynchronization scan found several equally plausible records and
    /// cannot choose between them.
    #[error("ambiguous payload record for {entry}: candidates at {candidates:?}")]
    AmbiguousPayloadRecord {
        /// Entry path.
        entry: String,
        /// Offsets of the equally scored candidates.
        candidates: Vec<u64>,
    },

    /// A `.Z` (Unix compress / LZW) stream is malformed.
    #[error("compress stream error: {message}")]
    Compress {
        /// Human-readable description.
        message: String,
    },

    /// A path from the distribution cannot be represented safely.
    #[error("unsafe or invalid path: {path}")]
    UnsafePath {
        /// The offending path.
        path: String,
    },
}

impl Error {
    /// Convenience constructor for [`Error::Io`].
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
