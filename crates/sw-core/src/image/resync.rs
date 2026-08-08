//! Resynchronization: locating a payload record whose expected offset is
//! wrong or unknown.
//!
//! Instead of searching for a bare filename (which may also appear inside
//! payload data), the scanner searches for the complete record prefix
//! `[u16 BE filename length][filename]`, trying the name variants `foo`,
//! `./foo` and `/foo`. Candidates are scored by distance to the expected
//! offset, with a bonus when the record that would *follow* the candidate
//! also matches its expected prefix. Several equally scored candidates are
//! reported as ambiguous instead of silently picking one.
use crate::error::{Error, Result};
use crate::image::latin1_bytes;
use crate::image::layout::IMAGE_HEADER_SIZE;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// How far before the expected offset a scan looks.
pub const DEFAULT_SEARCH_BACK: u64 = 1024 * 1024;

/// How far past the expected offset a scan looks.
pub const DEFAULT_SEARCH_FORWARD: u64 = 16 * 1024 * 1024;

/// Read chunk size used while scanning.
const SCAN_CHUNK: usize = 1024 * 1024;

/// What to search for.
#[derive(Debug)]
pub struct RecordSearch<'a> {
    /// The entry's raw IDB path; variants are derived automatically.
    pub raw_name: &'a str,
    /// Encoded payload size, used for the next-record cross-check. The
    /// cross-check is skipped when the size is unknown.
    pub encoded_size: Option<u64>,
    /// Raw name of the next payload record in the same image, if known.
    pub next_name: Option<&'a str>,
}

/// A record located by [`find_record`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundRecord {
    /// Offset of the record header within the archive.
    pub record_offset: u64,
    /// The name variant that matched, e.g. `./foo`.
    pub matched_name: String,
}

/// Searches an image archive for a payload record.
///
/// When `expected` is known the scan covers
/// `[expected - SEARCH_BACK, expected + SEARCH_FORWARD]`; otherwise the
/// whole archive is scanned from the end of the image header.
///
/// # Errors
///
/// Returns an I/O error if the archive cannot be read, and
/// [`Error::AmbiguousPayloadRecord`] when several candidates tie for the
/// best score — guessing would risk extracting the wrong bytes.
pub fn find_record(
    file: &mut File,
    file_size: u64,
    expected: Option<u64>,
    search: &RecordSearch<'_>,
) -> Result<Option<FoundRecord>> {
    let needles = record_prefixes(search.raw_name);
    if needles.is_empty() {
        return Ok(None);
    }
    let max_needle = needles
        .iter()
        .map(|(needle, _, _)| needle.len())
        .max()
        .unwrap_or(0);
    let (window_start, window_end) = match expected {
        Some(offset) => (
            offset.saturating_sub(DEFAULT_SEARCH_BACK),
            (offset + DEFAULT_SEARCH_FORWARD).min(file_size),
        ),
        None => (IMAGE_HEADER_SIZE.min(file_size), file_size),
    };

    // (offset, name byte length, display name)
    let mut candidates: Vec<(u64, usize, String)> = Vec::new();
    let mut chunk_start = window_start;
    // Previous chunk's tail, so prefixes spanning chunks are still found.
    let mut carry: Vec<u8> = Vec::new();

    while chunk_start < window_end {
        let length = ((window_end - chunk_start) as usize).min(SCAN_CHUNK);
        let mut buffer = vec![0u8; length];
        file.seek(SeekFrom::Start(chunk_start))
            .and_then(|_| file.read_exact(&mut buffer))
            .map_err(|source| Error::io("<image archive>", source))?;

        let mut haystack = std::mem::take(&mut carry);
        let base = chunk_start - haystack.len() as u64;
        haystack.extend_from_slice(&buffer);

        for (needle, byte_len, variant) in &needles {
            for at in find_all(&haystack, needle) {
                let offset = base + at as u64;
                if offset >= window_start && offset < window_end {
                    candidates.push((offset, *byte_len, variant.clone()));
                }
            }
        }

        let keep = max_needle.saturating_sub(1);
        carry = haystack[haystack.len().saturating_sub(keep)..].to_vec();
        chunk_start += length as u64;
    }

    // The chunk carry-over keeps `max_needle - 1` bytes, which can fully
    // contain a *shorter* name variant; that variant would then match the
    // same physical record once per chunk. Identical `(offset, variant)`
    // candidates are one record, not an ambiguity.
    candidates.sort();
    candidates.dedup();

    if candidates.is_empty() {
        return Ok(None);
    }

    // Score: next-record cross-check first, then distance to expected.
    let mut best: Vec<(bool, u64, FoundRecord)> = Vec::new();
    for (offset, byte_len, variant) in candidates {
        let next_matches = match (search.next_name, search.encoded_size) {
            (Some(next), Some(encoded_size)) => next_record_matches(
                file,
                file_size,
                offset + 2 + byte_len as u64 + encoded_size,
                next,
            )?,
            _ => false,
        };
        let distance = expected.map(|e| offset.abs_diff(e)).unwrap_or(0);
        let found = FoundRecord {
            record_offset: offset,
            matched_name: variant,
        };
        let score = (next_matches, std::cmp::Reverse(distance));
        match best.first() {
            None => best.push((next_matches, distance, found)),
            Some((b_next, b_distance, _)) => {
                let best_score = (*b_next, std::cmp::Reverse(*b_distance));
                if score > best_score {
                    best.clear();
                    best.push((next_matches, distance, found));
                } else if score == best_score {
                    best.push((next_matches, distance, found));
                }
            }
        }
    }

    match best.len() {
        0 => Ok(None),
        1 => Ok(best.pop().map(|(_, _, found)| found)),
        _ => Err(Error::AmbiguousPayloadRecord {
            entry: search.raw_name.to_string(),
            candidates: best.iter().map(|(_, _, f)| f.record_offset).collect(),
        }),
    }
}

/// Builds the record prefixes `[u16 BE length][name]` for every name
/// variant of `raw_name`, using the original Latin-1 archive bytes.
///
/// Returns `(prefix, name_byte_length, display_name)` triples.
fn record_prefixes(raw_name: &str) -> Vec<(Vec<u8>, usize, String)> {
    let name = latin1_bytes(raw_name);
    let mut variants: Vec<Vec<u8>> = vec![name.clone()];
    for prefix in [b"./".as_slice(), b"/".as_slice()] {
        let mut variant = prefix.to_vec();
        variant.extend_from_slice(&name);
        if !variants.contains(&variant) {
            variants.push(variant);
        }
    }
    variants
        .into_iter()
        .filter(|variant| variant.len() <= u16::MAX as usize)
        .map(|variant| {
            let byte_len = variant.len();
            let display = crate::image::latin1_string(&variant);
            let mut prefix = (variant.len() as u16).to_be_bytes().to_vec();
            prefix.extend_from_slice(&variant);
            (prefix, byte_len, display)
        })
        .collect()
}

/// Checks whether a record named `next_name` starts at `offset`.
fn next_record_matches(
    file: &mut File,
    file_size: u64,
    offset: u64,
    next_name: &str,
) -> Result<bool> {
    for (prefix, _, _) in record_prefixes(next_name) {
        if offset + prefix.len() as u64 > file_size {
            continue;
        }
        let mut buffer = vec![0u8; prefix.len()];
        file.seek(SeekFrom::Start(offset))
            .and_then(|_| file.read_exact(&mut buffer))
            .map_err(|source| Error::io("<image archive>", source))?;
        if buffer == prefix {
            return Ok(true);
        }
    }
    Ok(false)
}

/// All positions of `needle` in `haystack`.
fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    haystack
        .windows(needle.len())
        .enumerate()
        .filter_map(|(i, window)| (window == needle).then_some(i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A record whose short name-variant prefix sits fully inside the
    /// chunk carry-over must still be reported exactly once — before
    /// candidate deduplication this produced a bogus
    /// [`Error::AmbiguousPayloadRecord`].
    #[test]
    fn record_at_chunk_boundary_is_found_once() {
        let path = std::env::temp_dir().join(format!("sw-core-resync-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);

        // The longest needle for "foo" is "./foo" (7 bytes), so the carry
        // keeps 6 bytes; the 5-byte raw prefix `[00 03]foo` placed at
        // `SCAN_CHUNK - 5` lies entirely inside that carry.
        let record_offset = SCAN_CHUNK as u64 - 5;
        let mut bytes = vec![0u8; SCAN_CHUNK + 1024];
        bytes[record_offset as usize..record_offset as usize + 5]
            .copy_from_slice(&[0, 3, b'f', b'o', b'o']);
        {
            let mut file = File::create(&path).unwrap();
            file.write_all(&bytes).unwrap();
        }

        let mut file = File::open(&path).unwrap();
        let found = find_record(
            &mut file,
            bytes.len() as u64,
            None,
            &RecordSearch {
                raw_name: "foo",
                encoded_size: None,
                next_name: None,
            },
        )
        .unwrap()
        .expect("record must be found exactly once");
        assert_eq!(found.record_offset, record_offset);
        assert_eq!(found.matched_name, "foo");

        let _ = std::fs::remove_file(&path);
    }
}
