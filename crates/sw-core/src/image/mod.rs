//! Image archives: the payload containers of a distribution.
//!
//! An image (e.g. `eoe.sw`) is both a logical grouping of subsystems and a
//! physical file inside the distribution directory. The physical record
//! framing implemented here was reverse engineered from real IRIX
//! 4.0.5 – 6.5 media; it is deliberately isolated in this module so the
//! logical model (product / image / subsystem / entry) does not depend on
//! it.
pub(crate) mod layout;
pub(crate) mod resync;

use crate::compress;
use crate::descriptor::model::{Subsystem, Version};
use crate::error::Result;
use crate::mach::HardwareExpr;
use crate::names::ImageName;
use std::path::PathBuf;

/// Encodes a Latin-1 decoded string back to its original archive bytes.
pub(crate) fn latin1_bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u32 as u8).collect()
}

/// Decodes archive bytes into a string, one byte per character.
pub(crate) fn latin1_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// A logical image: a group of subsystems plus its physical archive.
#[derive(Debug, Clone)]
pub struct Image {
    /// Image name, e.g. `eoe.sw`.
    pub name: ImageName,
    /// Human-readable title from the descriptor.
    pub title: Option<String>,
    /// Product version carried by the image, inherited by its subsystems.
    pub version: Option<Version>,
    /// Installation order hint.
    pub order: Option<i32>,
    /// Hardware applicability expressions (OR-ed).
    pub mach: Vec<HardwareExpr>,
    /// The physical archive file.
    pub archive: ImageArchive,
    /// Subsystems grouped in this image.
    pub subsystems: Vec<Subsystem>,
}

/// The physical archive file of an image, e.g. `dist/eoe.sw`.
#[derive(Debug, Clone)]
pub struct ImageArchive {
    /// Path to the archive file.
    pub path: PathBuf,
    /// File size in bytes.
    pub file_size: u64,
    /// Computed record layout.
    pub layout: ImageLayout,
}

/// The computed physical layout of an image archive.
#[derive(Debug, Clone)]
pub struct ImageLayout {
    /// Size of the archive header in bytes.
    pub header_size: u64,
    /// Locators of all payload records, in IDB order.
    pub payloads: Vec<PayloadLocator>,
}

/// Where one entry's bytes live inside an image archive.
///
/// The offset is *expected*: it is derived from the archive layout
/// algorithm, and the actual record position is only confirmed when the
/// record is read (see [`reader`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadLocator {
    /// The image archive holding the payload.
    pub image: ImageName,
    /// Size of the payload as stored (compressed size, or plain size when
    /// stored uncompressed), or `None` when it could not be determined —
    /// zero is a legitimate empty payload and never means "unknown".
    pub encoded_size: Option<u64>,
    /// Expected record offset, or `None` when the layout could not be
    /// computed past an earlier record of unknown size.
    pub expected_record_offset: Option<u64>,
}

/// A payload read from an image archive.
#[derive(Debug, Clone)]
pub struct Payload {
    /// The payload bytes, still compressed if the entry is stored
    /// compressed.
    pub bytes: Vec<u8>,
    /// Where the payload was actually found.
    pub location: PayloadLocation,
    /// Whether the entry is stored compressed (its `cmpsize` is non-zero).
    ///
    /// This — not the `1F 9D` magic — decides decoding: a distribution can
    /// legitimately ship a `.Z` file *uncompressed* (`cmpsize(0)`), and
    /// such a payload starts with the magic but must be delivered as-is.
    pub stored_compressed: bool,
}

impl Payload {
    /// Decodes the payload, transparently decompressing `.Z` data.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::Error::Compress`] on malformed compressed
    /// data.
    pub fn decode(&self) -> Result<Vec<u8>> {
        if self.stored_compressed {
            compress::decompress(&self.bytes)
        } else {
            Ok(self.bytes.clone())
        }
    }
}

/// Where a payload record was actually found in an image archive.
#[derive(Debug, Clone)]
pub struct PayloadLocation {
    /// The image archive read from.
    pub image: ImageName,
    /// The offset predicted by the layout algorithm, if any.
    pub expected_record_offset: Option<u64>,
    /// The offset the record was actually found at.
    pub actual_record_offset: u64,
    /// Offset of the payload data itself (after the record header).
    pub data_offset: u64,
    /// The record name variant that matched, e.g. `./foo`.
    pub matched_name: String,
    /// How the record was located.
    pub resolution: PayloadResolution,
}

/// How a payload record was located, so callers can surface recovery
/// instead of silently fixing offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadResolution {
    /// Found exactly at the expected offset.
    Exact,
    /// Found at `expected + delta` without a search.
    Delta {
        /// Distance from the expected offset.
        delta: i64,
    },
    /// Found by a resynchronization scan around the expected offset.
    Resynced {
        /// Distance from the expected offset, if one was known.
        delta: Option<i64>,
    },
    /// Found by scanning because no expected offset existed.
    Scanned,
}
