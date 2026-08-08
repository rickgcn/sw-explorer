//! Reads payload records from image archives.
//!
//! The reader is a session: archive files are opened once and cached, and
//! each image carries a running *delta hint* — the offset error observed
//! on the last resynchronization. On intact media every read hits the
//! expected offset; on media with a systematic offset shift, one resync
//! teaches the reader the delta and subsequent reads hit
//! `expected + delta` immediately instead of scanning again.
//!
//! Reading is always verification: the record name must match the entry's
//! raw path, and the outcome is reported honestly through
//! [`crate::image::PayloadResolution`].
use crate::distribution::Distribution;
use crate::error::{Error, Result};
use crate::idb::Entry;
use crate::image::latin1_bytes;
use crate::image::resync::{self, RecordSearch};
use crate::image::{ImageArchive, Payload, PayloadLocation, PayloadResolution};
use crate::names::ImageName;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// Reads payloads of the entries of one distribution.
pub struct ImageReader<'a> {
    distribution: &'a Distribution,
    states: HashMap<ImageName, ImageState>,
}

/// Per-image session state.
struct ImageState {
    file: File,
    file_size: u64,
    /// Offset error learned from the last resync on this image.
    delta: i64,
}

impl<'a> ImageReader<'a> {
    pub(crate) fn new(distribution: &'a Distribution) -> Self {
        ImageReader {
            distribution,
            states: HashMap::new(),
        }
    }

    /// Reads the payload of one entry.
    ///
    /// The returned bytes are still compressed if the entry is stored
    /// compressed; call [`Payload::decode`] to transparently decompress.
    ///
    /// # Errors
    ///
    /// * [`Error::PayloadNotFound`] — the entry carries no payload locator.
    /// * [`Error::ProductNotFound`] — the locator's product is unknown.
    /// * [`Error::PayloadSizeUnknown`] — the stored size is unknown, so the
    ///   payload cannot be delimited.
    /// * [`Error::ResyncFailed`] — the record could not be located.
    /// * [`Error::AmbiguousPayloadRecord`] — several records tied.
    /// * [`Error::Io`] — the archive could not be read.
    pub fn read(&mut self, entry: &Entry) -> Result<Payload> {
        let locator = entry
            .payload
            .as_ref()
            .ok_or_else(|| Error::PayloadNotFound {
                entry: entry.path.to_string(),
            })?
            .clone();

        let product_name = locator.image.product().as_str();
        let product =
            self.distribution
                .product(product_name)
                .ok_or_else(|| Error::ProductNotFound {
                    name: product_name.to_string(),
                })?;
        let archive: &ImageArchive = &product
            .images
            .iter()
            .find(|image| image.name == locator.image)
            .ok_or_else(|| Error::PayloadNotFound {
                entry: entry.path.to_string(),
            })?
            .archive;

        let image = locator.image.clone();
        if !self.states.contains_key(&image) {
            let file =
                File::open(&archive.path).map_err(|source| Error::io(&archive.path, source))?;
            self.states.insert(
                image.clone(),
                ImageState {
                    file,
                    file_size: archive.file_size,
                    delta: 0,
                },
            );
        }

        let expected_name = latin1_bytes(&entry.raw_path);

        // 1. With a learned delta, probe `expected + delta` first.
        let probe = {
            let state = self.states.get_mut(&image).expect("inserted above");
            let mut hit = None;
            if state.delta != 0
                && let Some(expected) = locator.expected_record_offset
                && let Some(offset) = expected.checked_add_signed(state.delta)
                && matches!(
                    read_record_name(&mut state.file, state.file_size, offset)?,
                    Some(name) if name == expected_name
                )
            {
                hit = Some((offset, state.delta));
            }
            hit
        };
        if let Some((offset, delta)) = probe {
            return self.finish_read(
                entry,
                &locator,
                offset,
                entry.raw_path.clone(),
                PayloadResolution::Delta { delta },
            );
        }

        // 2. Try the expected offset itself.
        let exact = {
            let state = self.states.get_mut(&image).expect("inserted above");
            let mut hit = None;
            if let Some(expected) = locator.expected_record_offset
                && matches!(
                    read_record_name(&mut state.file, state.file_size, expected)?,
                    Some(name) if name == expected_name
                )
            {
                hit = Some(expected);
            }
            hit
        };
        if let Some(offset) = exact {
            return self.finish_read(
                entry,
                &locator,
                offset,
                entry.raw_path.clone(),
                PayloadResolution::Exact,
            );
        }

        // 3. Resynchronize, using the next payload-bearing entry of the
        //    same image as a cross-check hint, and learn the delta.
        let next_name = next_payload_name(&product.entries, entry);
        let found = {
            let state = self.states.get_mut(&image).expect("inserted above");
            resync::find_record(
                &mut state.file,
                state.file_size,
                locator.expected_record_offset,
                &RecordSearch {
                    raw_name: &entry.raw_path,
                    encoded_size: locator.encoded_size,
                    next_name: next_name.as_deref(),
                },
            )?
        }
        .ok_or_else(|| Error::ResyncFailed {
            entry: entry.path.to_string(),
        })?;

        let resolution = match locator.expected_record_offset {
            Some(expected) => {
                let delta = found.record_offset as i64 - expected as i64;
                self.states.get_mut(&image).expect("inserted above").delta = delta;
                PayloadResolution::Resynced { delta: Some(delta) }
            }
            None => PayloadResolution::Scanned,
        };
        self.finish_read(
            entry,
            &locator,
            found.record_offset,
            found.matched_name,
            resolution,
        )
    }

    /// Reads the payload bytes of a located record and builds the result.
    fn finish_read(
        &mut self,
        entry: &Entry,
        locator: &crate::image::PayloadLocator,
        record_offset: u64,
        matched_name: String,
        resolution: PayloadResolution,
    ) -> Result<Payload> {
        let name_length = latin1_bytes(&matched_name).len() as u64;
        let data_offset = record_offset + 2 + name_length;
        let Some(encoded_size) = locator.encoded_size else {
            return Err(Error::PayloadSizeUnknown {
                entry: entry.path.to_string(),
            });
        };
        let image = locator.image.clone();
        let state = self.states.get_mut(&image).expect("inserted above");
        let mut bytes = vec![0u8; encoded_size as usize];
        state
            .file
            .seek(SeekFrom::Start(data_offset))
            .and_then(|_| state.file.read_exact(&mut bytes))
            .map_err(|source| Error::io("<image archive>", source))?;
        Ok(Payload {
            bytes,
            stored_compressed: entry.compressed_size().is_some_and(|size| size > 0),
            location: PayloadLocation {
                image,
                expected_record_offset: locator.expected_record_offset,
                actual_record_offset: record_offset,
                data_offset,
                matched_name,
                resolution,
            },
        })
    }
}

/// Reads the raw record name bytes at `offset`, if a plausible record
/// starts there.
fn read_record_name(file: &mut File, file_size: u64, offset: u64) -> Result<Option<Vec<u8>>> {
    if offset + 2 > file_size {
        return Ok(None);
    }
    let mut length = [0u8; 2];
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(&mut length))
        .map_err(|source| Error::io("<image archive>", source))?;
    let length = u16::from_be_bytes(length) as u64;
    if length == 0 || offset + 2 + length > file_size {
        return Ok(None);
    }
    let mut name = vec![0u8; length as usize];
    file.read_exact(&mut name)
        .map_err(|source| Error::io("<image archive>", source))?;
    Ok(Some(name))
}

/// Finds the raw path of the next payload-bearing entry in the same image.
fn next_payload_name(entries: &[Entry], entry: &Entry) -> Option<String> {
    let image = entry.payload.as_ref()?.image.clone();
    entries
        .iter()
        .skip_while(|candidate| candidate.id != entry.id)
        .skip(1)
        .find(|candidate| {
            candidate
                .payload
                .as_ref()
                .is_some_and(|locator| locator.image == image)
        })
        .map(|candidate| candidate.raw_path.clone())
}
