//! Parser for binary product descriptors.
//!
//! Verified facts about the format, from real IRIX 4.0.5 – 6.5 media:
//!
//! * the file starts with a 13-byte magic `pd001V<version>P<xx>` followed
//!   by a NUL byte, mirroring the `im001…` image archive header;
//! * the body is a binary, length-prefixed record structure whose grammar
//!   has not been reverse engineered yet.
//!
//! Until the body grammar is known, [`parse`] decodes the header and
//! reports the undecoded remainder as a diagnostic.
use crate::descriptor::model::{DescriptorHeader, ProductDescriptor};
use crate::diagnostic::Diagnostic;
use crate::error::{Error, Result};
use std::path::Path;

/// Length of the descriptor header including its trailing NUL.
pub const DESCRIPTOR_HEADER_SIZE: usize = 13;

/// Parses a product descriptor file.
///
/// # Errors
///
/// Returns [`Error::Descriptor`] if the file is shorter than the header,
/// does not start with the `pd001V` magic, or the header is not
/// NUL-terminated.
pub fn parse(
    bytes: &[u8],
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<ProductDescriptor> {
    if bytes.len() < DESCRIPTOR_HEADER_SIZE {
        return Err(Error::Descriptor {
            path: path.to_path_buf(),
            message: "file shorter than descriptor header".to_string(),
        });
    }
    if !bytes.starts_with(b"pd001V") {
        return Err(Error::Descriptor {
            path: path.to_path_buf(),
            message: "bad descriptor magic".to_string(),
        });
    }
    if bytes[DESCRIPTOR_HEADER_SIZE - 1] != 0 {
        return Err(Error::Descriptor {
            path: path.to_path_buf(),
            message: "descriptor header is not NUL-terminated".to_string(),
        });
    }

    let magic = String::from_utf8_lossy(&bytes[..DESCRIPTOR_HEADER_SIZE - 1]).into_owned();
    diagnostics.push(Diagnostic::warning(
        "binary descriptor body is not decoded; titles, versions, flags and rules are unavailable",
        Some(path.display().to_string()),
    ));

    Ok(ProductDescriptor {
        header: DescriptorHeader { magic },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_magic() {
        let bytes = b"pd001V530P00\0body-bytes";
        let mut diagnostics = Vec::new();
        let descriptor = parse(bytes, Path::new("eoe1"), &mut diagnostics).unwrap();
        assert_eq!(descriptor.header.magic, "pd001V530P00");
        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn rejects_bad_input() {
        let mut diagnostics = Vec::new();
        assert!(parse(b"short", Path::new("x"), &mut diagnostics).is_err());
        assert!(parse(b"im001V530P00\0x", Path::new("x"), &mut diagnostics).is_err());
        assert!(parse(b"pd001V530P00Xx", Path::new("x"), &mut diagnostics).is_err());
    }
}
