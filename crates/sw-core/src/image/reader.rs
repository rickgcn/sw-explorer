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
//!
//! Entries arrive as canonical [`EntryKey`] values, which the reader
//! resolves against the distribution it is bound to: the owning product
//! and the exact logical image an entry is attached to come from that
//! resolution, never from the qualified product segment of a name — an
//! IDB record may legitimately name a *foreign* subsystem whose product
//! has no presence in the distribution at all. Because the reader mints
//! every [`LocatedEntry`] itself, one distribution's identity can never
//! be paired with another distribution's entry metadata.
use crate::distribution::{Distribution, EntryKey, LocatedEntry, Product};
use crate::error::{Error, Result};
use crate::idb::{Entry, EntryId};
use crate::image::latin1_bytes;
use crate::image::resync::{self, RecordSearch};
use crate::image::{Image, ImageArchive, Payload, PayloadLocation, PayloadResolution};
use crate::names::ImageName;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// Reads payloads of the entries of one distribution.
pub struct ImageReader<'a> {
    distribution: &'a Distribution,
    // Session state per *physical* archive file (its qualified file
    // name), not per logical image: several logical images may
    // reference the same physical archive, and they legitimately share
    // the open file and the learned delta.
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

    /// Resolves a canonical key through the distribution this reader is
    /// bound to.
    ///
    /// The reader mints every [`LocatedEntry`] it acts on itself, so a
    /// caller can never hand it one distribution's identity paired with
    /// another distribution's entry metadata. Keys originating from
    /// another distribution are outside the API contract (see
    /// [`crate::distribution::EntryKey`]): when their numeric halves
    /// happen to be valid here, they identify an entry of *this*
    /// distribution.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EntryNotFound`] when the key does not resolve in
    /// the distribution this reader is bound to.
    pub(crate) fn locate(&self, key: EntryKey) -> Result<LocatedEntry<'a>> {
        self.distribution
            .entry(key)
            .ok_or(Error::EntryNotFound { key })
    }

    /// Reads the payload of one entry.
    ///
    /// The key is resolved against the distribution this reader is
    /// bound to: the entry's canonical identity decides the exact
    /// logical image it is attached to, and that image's archive is the
    /// physical file read; the qualified product segment of the payload
    /// locator's image name is never used to guess an owning product.
    /// Keys originating from another distribution are outside the API
    /// contract (see the stability contract on
    /// [`crate::distribution::EntryKey`]): when their numeric halves
    /// happen to be valid here, they identify an entry of *this*
    /// distribution.
    ///
    /// The returned bytes are still compressed if the entry is stored
    /// compressed; call [`Payload::decode`] to transparently decompress.
    ///
    /// # Errors
    ///
    /// * [`Error::EntryNotFound`] — the key does not resolve in the
    ///   distribution this reader is bound to.
    /// * [`Error::PayloadNotFound`] — the entry carries no payload
    ///   locator, or its attachment does not resolve.
    /// * [`Error::PayloadSizeUnknown`] — the stored size is unknown, so the
    ///   payload cannot be delimited.
    /// * [`Error::ResyncFailed`] — the record could not be located.
    /// * [`Error::AmbiguousPayloadRecord`] — several records tied.
    /// * [`Error::Io`] — the archive could not be read.
    pub fn read(&mut self, key: EntryKey) -> Result<Payload> {
        let entry = self.locate(key)?;
        let locator = entry
            .entry
            .payload
            .as_ref()
            .ok_or_else(|| Error::PayloadNotFound {
                entry: entry.entry.path.to_string(),
            })?
            .clone();

        // The exact logical image the entry is attached to, confirmed
        // by attachment membership — never a name lookup. A record
        // naming a foreign subsystem is served by the synthetic image
        // of its *owning* product.
        let located_image = self
            .distribution
            .image_containing_entry(entry.key)
            .ok_or_else(|| Error::PayloadNotFound {
                entry: entry.entry.path.to_string(),
            })?;
        let archive: &ImageArchive = &located_image.image.archive;

        let image = located_image.image.name.clone();
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

        let expected_name = latin1_bytes(&entry.entry.raw_path);

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
                entry.entry,
                &locator,
                offset,
                entry.entry.raw_path.clone(),
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
                entry.entry,
                &locator,
                offset,
                entry.entry.raw_path.clone(),
                PayloadResolution::Exact,
            );
        }

        // 3. Resynchronize, using the next payload-bearing entry of the
        //    same image as a cross-check hint, and learn the delta.
        let product = self
            .distribution
            .products()
            .get(entry.key.product_index)
            .ok_or_else(|| Error::PayloadNotFound {
                entry: entry.entry.path.to_string(),
            })?;
        let next_name = next_payload_name(product, located_image.image, entry.entry);
        let found = {
            let state = self.states.get_mut(&image).expect("inserted above");
            resync::find_record(
                &mut state.file,
                state.file_size,
                locator.expected_record_offset,
                &RecordSearch {
                    raw_name: &entry.entry.raw_path,
                    encoded_size: locator.encoded_size,
                    next_name: next_name.as_deref(),
                },
            )?
        }
        .ok_or_else(|| Error::ResyncFailed {
            entry: entry.entry.path.to_string(),
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
            entry.entry,
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

/// Finds the raw path of the next payload-bearing entry attached to the
/// same exact image, scanning the owning product's entries in IDB
/// order.
///
/// Membership is the image's recorded attachment (`entry_ids`), never a
/// qualified-name comparison: a record naming a foreign subsystem finds
/// its hint in its *owning* product's tree.
fn next_payload_name(product: &Product, image: &Image, entry: &Entry) -> Option<String> {
    let membership: std::collections::HashSet<EntryId> = image
        .subsystems
        .iter()
        .flat_map(|subsystem| subsystem.entry_ids.iter().copied())
        .collect();
    product
        .entries
        .iter()
        .skip_while(|candidate| candidate.id != entry.id)
        .skip(1)
        .find(|candidate| candidate.payload.is_some() && membership.contains(&candidate.id))
        .map(|candidate| candidate.raw_path.clone())
}
