//! Construction of the entry DTOs declared by the CXX bridge from
//! `sw-core` entries.
//!
//! The conversions stay faithful to the domain model:
//!
//! * "unknown" (attribute absent) is never conflated with a zero
//!   value: every optional field carries an explicit `*_known` flag;
//! * the stored size follows `Entry::encoded_size()` archive
//!   semantics: `cmpsize` when non-zero, `size` when stored
//!   uncompressed — a `cmpsize(0)` record never surfaces as
//!   "stored size 0";
//! * unknown file type letters and unknown `config(...)` values are
//!   preserved verbatim;
//! * unresolved `mach(...)` payloads are kept next to the parsed
//!   expressions, never dropped;
//! * the payload locator is metadata only: nothing here reads an
//!   image archive, and the expected record offset is passed on as
//!   exactly that — expected, unverified.
use crate::bridge::ffi;
use sw_core::idb::attribute::{ConfigMode, IdbAttribute};
use sw_core::idb::{Entry, FileType};

/// Maps a file type to the bridge enum plus the raw IDB type letter;
/// the letter survives even when the type itself is unknown.
fn file_type_parts(file_type: FileType) -> (ffi::EntryFileType, String) {
    let (kind, letter) = match file_type {
        FileType::Regular => (ffi::EntryFileType::Regular, 'f'),
        FileType::Directory => (ffi::EntryFileType::Directory, 'd'),
        FileType::SymbolicLink => (ffi::EntryFileType::SymbolicLink, 'l'),
        FileType::BlockDevice => (ffi::EntryFileType::BlockDevice, 'b'),
        FileType::CharacterDevice => (ffi::EntryFileType::CharacterDevice, 'c'),
        FileType::Fifo => (ffi::EntryFileType::Fifo, 'p'),
        FileType::Unknown(letter) => (ffi::EntryFileType::Other, letter),
    };
    (kind, letter.to_string())
}

/// The parsed and the unresolved `mach(...)` payloads of an entry,
/// side by side; unresolved payloads are never dropped.
fn mach_lists(entry: &Entry) -> (Vec<String>, Vec<String>) {
    let parsed = entry
        .mach()
        .map(|expression| expression.raw.clone())
        .collect();
    let unresolved = entry
        .attributes
        .iter()
        .filter_map(|attribute| match attribute {
            IdbAttribute::MachUnparsed { raw, .. } => Some(raw.clone()),
            _ => None,
        })
        .collect();
    (parsed, unresolved)
}

/// `sum(...)` checksum attribute, if present.
fn checksum(entry: &Entry) -> Option<u64> {
    entry
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            IdbAttribute::Checksum(sum) => Some(*sum),
            _ => None,
        })
}

/// Renders a config mode; unknown values are preserved verbatim.
fn config_mode_text(mode: &ConfigMode) -> String {
    match mode {
        ConfigMode::Suggest => "suggest".to_string(),
        ConfigMode::NoUpdate => "noupdate".to_string(),
        ConfigMode::Update => "update".to_string(),
        ConfigMode::Unknown(raw) => raw.clone(),
    }
}

/// Builds the summary DTO of one entry; `product_id` is the object id
/// of the containing product.
pub(crate) fn entry_summary(product_id: u64, entry: &Entry) -> ffi::EntrySummary {
    let (file_type, file_type_raw) = file_type_parts(entry.file_type);
    let (mach, unresolved_mach) = mach_lists(entry);
    ffi::EntrySummary {
        product_id,
        entry_id: entry.id.0 as u64,
        path: entry.path.to_string(),
        subsystem: entry.subsystem.to_string(),
        file_type,
        file_type_raw,
        size_known: entry.size().is_some(),
        size: entry.size().unwrap_or(0),
        stored_size_known: entry.encoded_size().is_some(),
        stored_size: entry.encoded_size().unwrap_or(0),
        mach,
        unresolved_mach,
    }
}

/// Builds the detail DTO of one entry; `product_id` is the object id
/// of the containing product.
pub(crate) fn entry_detail(product_id: u64, entry: &Entry) -> ffi::EntryDetail {
    let (file_type, file_type_raw) = file_type_parts(entry.file_type);
    let (mach, unresolved_mach) = mach_lists(entry);
    let checksum = checksum(entry);
    let config_mode = entry.config_mode();
    let symlink_target = entry.symlink_target();
    let device = entry.device();
    ffi::EntryDetail {
        product_id,
        entry_id: entry.id.0 as u64,
        file_type,
        file_type_raw,
        mode: entry.mode,
        owner: entry.owner.clone(),
        group: entry.group.clone(),
        path: entry.path.to_string(),
        raw_path: entry.raw_path.clone(),
        source_path: entry.source_path.clone(),
        subsystem: entry.subsystem.to_string(),
        size_known: entry.size().is_some(),
        size: entry.size().unwrap_or(0),
        compressed_size_known: entry.compressed_size().is_some(),
        compressed_size: entry.compressed_size().unwrap_or(0),
        stored_size_known: entry.encoded_size().is_some(),
        stored_size: entry.encoded_size().unwrap_or(0),
        checksum_known: checksum.is_some(),
        checksum: checksum.unwrap_or(0),
        config_known: config_mode.is_some(),
        config_mode: config_mode
            .as_ref()
            .map(config_mode_text)
            .unwrap_or_default(),
        symlink_target_known: symlink_target.is_some(),
        symlink_target: symlink_target.unwrap_or_default().to_string(),
        device_known: device.is_some(),
        device_major: device.map(|(major, _)| major).unwrap_or(0),
        device_minor: device.map(|(_, minor)| minor).unwrap_or(0),
        mach,
        unresolved_mach,
        payload_present: entry.payload.is_some(),
        payload_image: entry
            .payload
            .as_ref()
            .map(|locator| locator.image.file_name())
            .unwrap_or_default(),
        payload_encoded_size_known: entry
            .payload
            .as_ref()
            .is_some_and(|locator| locator.encoded_size.is_some()),
        payload_encoded_size: entry
            .payload
            .as_ref()
            .and_then(|locator| locator.encoded_size)
            .unwrap_or(0),
        expected_record_offset_known: entry
            .payload
            .as_ref()
            .is_some_and(|locator| locator.expected_record_offset.is_some()),
        expected_record_offset: entry
            .payload
            .as_ref()
            .and_then(|locator| locator.expected_record_offset)
            .unwrap_or(0),
        origin_idb_path: entry.origin.idb_path.display().to_string(),
        origin_line: entry.origin.line_number as u64,
        raw_idb_line: entry.origin.raw_line.clone(),
    }
}
