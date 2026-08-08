//! The image archive layout algorithm.
//!
//! Verified against all payload files of real IRIX 4.0.5, 5.3, 6.2, 6.3
//! and 6.5 distributions (every archive walked exactly to its end):
//!
//! * the archive starts with a 13-byte header (`im001V…` magic + NUL);
//! * each record is `[u16 BE filename length][filename][payload]`;
//! * records appear in IDB order;
//! * the payload length is `cmpsize` when non-zero, otherwise `size`
//!   (files with `cmpsize(0)` are stored uncompressed).
use crate::names::ImageName;

use super::latin1_bytes;

/// Size of the image archive header (`im001V530P00\0` and friends).
pub const IMAGE_HEADER_SIZE: u64 = 13;

/// Magic bytes an image archive header starts with.
const IMAGE_HEADER_MAGIC: &[u8] = b"im001V";

/// Size of a record header: a big-endian u16 filename length.
pub const RECORD_HEADER_SIZE: u64 = 2;

/// One record's worth of layout input.
#[derive(Debug, Clone)]
pub struct LayoutInput {
    /// The entry's raw IDB path, which is also the record filename.
    pub raw_name: String,
    /// The encoded payload size, if it could be determined.
    pub encoded_size: Option<u64>,
}

/// Computes expected record offsets for one image archive.
///
/// The returned vector parallels `inputs`. Once a record's encoded size is
/// unknown, the cursor cannot advance and every following record gets
/// `expected_record_offset: None` — the algorithm never guesses.
pub fn compute_layout(image: &ImageName, inputs: &[LayoutInput]) -> Vec<super::PayloadLocator> {
    let mut cursor = Some(IMAGE_HEADER_SIZE);
    inputs
        .iter()
        .map(|input| {
            let locator = super::PayloadLocator {
                image: image.clone(),
                encoded_size: input.encoded_size,
                expected_record_offset: cursor,
            };
            cursor = match (cursor, input.encoded_size) {
                (Some(offset), Some(size)) => Some(
                    offset + RECORD_HEADER_SIZE + latin1_bytes(&input.raw_name).len() as u64 + size,
                ),
                _ => None,
            };
            locator
        })
        .collect()
}

/// Validates the 13-byte header of the image archive at `path`.
///
/// A valid header is exactly the `im001V…` magic terminated by a NUL
/// byte, e.g. `im001V530P00\0`.
///
/// # Errors
///
/// Returns [`crate::error::Error::ImageFormat`] if the header does not
/// match, or [`crate::error::Error::Io`] if the file cannot be read.
pub fn validate_archive_header(path: &std::path::Path) -> crate::error::Result<()> {
    use std::io::Read;

    let mut header = [0u8; IMAGE_HEADER_SIZE as usize];
    let read = std::fs::File::open(path)
        .and_then(|mut file| file.read(&mut header))
        .map_err(|source| crate::error::Error::io(path, source))?;
    if read < IMAGE_HEADER_SIZE as usize {
        return Err(crate::error::Error::ImageFormat {
            path: path.to_path_buf(),
            message: format!("file is only {read} bytes, smaller than the 13-byte header"),
        });
    }
    if !header.starts_with(IMAGE_HEADER_MAGIC) {
        return Err(crate::error::Error::ImageFormat {
            path: path.to_path_buf(),
            message: "missing im001V magic".to_string(),
        });
    }
    if header[IMAGE_HEADER_SIZE as usize - 1] != 0 {
        return Err(crate::error::Error::ImageFormat {
            path: path.to_path_buf(),
            message: "header is not NUL-terminated".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::ProductName;

    fn image() -> ImageName {
        ImageName::from_parts(ProductName::new("eoe1").unwrap(), "sw".to_string()).unwrap()
    }

    #[test]
    fn computes_sequential_offsets() {
        // Mirrors the first two records of IRIX 5.3 eoe1.sw.
        let inputs = vec![
            LayoutInput {
                raw_name: ".bin.mv.sh".into(),
                encoded_size: Some(756),
            },
            LayoutInput {
                raw_name: ".cshrc".into(),
                encoded_size: Some(465),
            },
        ];
        let locators = compute_layout(&image(), &inputs);
        assert_eq!(locators[0].expected_record_offset, Some(13));
        assert_eq!(locators[1].expected_record_offset, Some(13 + 2 + 10 + 756));
    }

    #[test]
    fn unknown_size_poisons_following_offsets() {
        let inputs = vec![
            LayoutInput {
                raw_name: "a".into(),
                encoded_size: None,
            },
            LayoutInput {
                raw_name: "b".into(),
                encoded_size: Some(10),
            },
        ];
        let locators = compute_layout(&image(), &inputs);
        assert_eq!(locators[0].expected_record_offset, Some(13));
        assert_eq!(locators[1].expected_record_offset, None);
    }

    #[test]
    fn non_ascii_names_count_archive_bytes() {
        // `é` is one byte in the archive (Latin-1) but two in UTF-8; the
        // cursor must advance by the archive byte count.
        let inputs = vec![
            LayoutInput {
                raw_name: "café.txt".into(),
                encoded_size: Some(10),
            },
            LayoutInput {
                raw_name: "next".into(),
                encoded_size: Some(5),
            },
        ];
        let locators = compute_layout(&image(), &inputs);
        assert_eq!(locators[1].expected_record_offset, Some(13 + 2 + 8 + 10));
    }
}
