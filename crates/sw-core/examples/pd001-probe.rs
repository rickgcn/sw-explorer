//! Research probe for the binary product descriptor (`pd001`) body.
//!
//! This is a reverse-engineering workbench, kept deliberately separate
//! from the library's descriptor parser. The grammar it implements was
//! established against the whole descriptor corpus:
//!
//! * stream header `07c4 0001 07c3` + layout level (5/6 vs 8/9 family);
//! * big-endian u16/u32 and length-prefixed (`BE16` + bytes) strings;
//! * product record: name, description, flags, stamp, metadata blobs
//!   (level 8/9), image count;
//! * image record: flags, short name, description, two unknown u16
//!   fields, version, then a level-dependent tail (level 5 has two
//!   extra unknown u16 fields, level 6 has neither metadata nor
//!   reserved fields), metadata blobs, subsystem count;
//! * subsystem record: flags, short name, description, mapping string;
//! * rules region: exactly as many `BE16 count + records` slots as the
//!   layout level number, with a fixed shape per slot (flat range
//!   records, clause-of-ranges, or plain blob lists).
//!
//! Validation: parsed product/image/subsystem names and counts are
//! compared against the IDB-derived hierarchy of the same product, and
//! exact byte consumption is checked per descriptor. Where the grammar
//! does not hold, the probe records the exact offset plus raw context
//! bytes instead of guessing, and tries to resynchronize on the next
//! expected record boundary.
//!
//! Usage:
//!
//! ```sh
//! cargo run -p sw-core --example pd001-probe -- --json probe.json <dist-dir>...
//! ```

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use sw_core::distribution::Distribution;

/// Descriptor header length in bytes, including the trailing NUL.
const HEADER_SIZE: usize = 13;

/// Sanity limit for section counts in the rules walk. Large core
/// products legitimately replace hundreds of legacy subsystems.
const MAX_SECTION_COUNT: u16 = 4096;

/// Decodes bytes into a string, one character per byte.
fn latin1_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Lowercase hex encoding.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A parse stop with the exact body offset.
#[derive(Debug)]
struct Stop {
    offset: usize,
    message: String,
}

fn stop(offset: usize, message: impl Into<String>) -> Stop {
    Stop {
        offset,
        message: message.into(),
    }
}

/// Big-endian cursor over the descriptor body.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Cursor { bytes, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn be16(&mut self, what: &str) -> Result<u16, Stop> {
        if self.remaining() < 2 {
            return Err(stop(self.pos, format!("truncated reading {what}")));
        }
        let value = u16::from_be_bytes([self.bytes[self.pos], self.bytes[self.pos + 1]]);
        self.pos += 2;
        Ok(value)
    }

    fn be32(&mut self, what: &str) -> Result<u32, Stop> {
        if self.remaining() < 4 {
            return Err(stop(self.pos, format!("truncated reading {what}")));
        }
        let value = u32::from_be_bytes([
            self.bytes[self.pos],
            self.bytes[self.pos + 1],
            self.bytes[self.pos + 2],
            self.bytes[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(value)
    }

    /// Reads a `BE16 length + bytes` blob, one character per byte.
    fn lp16(&mut self, what: &str) -> Result<String, Stop> {
        let len = self.be16(what)? as usize;
        if self.remaining() < len {
            return Err(stop(
                self.pos - 2,
                format!(
                    "{what} length {len} exceeds {} remaining bytes",
                    self.remaining()
                ),
            ));
        }
        let text = latin1_string(&self.bytes[self.pos..self.pos + len]);
        self.pos += len;
        Ok(text)
    }

    /// Reads an LP16 blob with an upper length bound.
    fn lp16_max(&mut self, what: &str, max: usize) -> Result<String, Stop> {
        let len = self.be16(what)? as usize;
        if len > max {
            return Err(stop(
                self.pos - 2,
                format!("{what} length {len} exceeds limit {max}"),
            ));
        }
        if self.remaining() < len {
            return Err(stop(
                self.pos - 2,
                format!(
                    "{what} length {len} exceeds {} remaining bytes",
                    self.remaining()
                ),
            ));
        }
        let text = latin1_string(&self.bytes[self.pos..self.pos + len]);
        self.pos += len;
        Ok(text)
    }

    /// Bytes at `offset`, for boundary checks.
    fn at(&self, offset: usize) -> &[u8] {
        &self.bytes[offset.min(self.bytes.len())..]
    }

    /// Up to `n` bytes of hex context at `offset`.
    fn context(&self, offset: usize, n: usize) -> String {
        let start = offset.min(self.bytes.len());
        hex(&self.bytes[start..self.bytes.len().min(start + n)])
    }
}

/// A located parse problem.
#[derive(Serialize)]
struct Failure {
    offset: usize,
    message: String,
    context_hex: String,
}

/// One `(LP16, LP16, LP16, BE32 low, BE32 high)` record from the
/// tentative rules walk.
#[derive(Serialize)]
struct RangeRecordProbe {
    offset: usize,
    fields: [String; 3],
    low: u32,
    high: u32,
    /// Which clause of a clause-shaped section this record came from.
    clause_index: Option<u32>,
}

/// One walked rules section.
#[derive(Serialize)]
struct SectionProbe {
    offset: usize,
    count: u16,
    /// `ranges` for flat range records, `clauses` for the nested
    /// clause-of-ranges shape, `strings` for a plain LP16 blob list,
    /// `empty` for a zero count.
    kind: String,
    records: Vec<RangeRecordProbe>,
    /// Blob contents when `kind` is `strings`.
    blobs: Vec<String>,
}

/// One parsed subsystem record.
#[derive(Serialize)]
struct SubsystemProbe {
    offset: usize,
    expected_name: Option<String>,
    flags: u16,
    name: String,
    name_matches_idb: bool,
    description: String,
    mapping: String,
    /// Sections the tentative walk consumed, in order.
    sections: Vec<SectionProbe>,
    /// Where the tentative walk stopped, if it did not reach the next
    /// record boundary cleanly.
    walk_stop: Option<Failure>,
    /// Raw bytes between the walk stop and the next boundary.
    unknown_tail_hex: Option<String>,
    unknown_tail_len: usize,
}

/// One parsed image record.
#[derive(Serialize)]
struct ImageProbe {
    offset: usize,
    expected_name: Option<String>,
    flags: u16,
    name: String,
    name_matches_idb: bool,
    description: String,
    /// Unknown u16 between description and order candidate.
    field_a: u16,
    /// Strong installation-order candidate.
    order_candidate: u16,
    version: u32,
    /// Unknown u16 fields in the image tail (layout level 5 only).
    tail_unknowns: Vec<u16>,
    metadata: Vec<String>,
    subsystem_count: u32,
    subsystems: Vec<SubsystemProbe>,
}

/// Probe result of one descriptor.
#[derive(Serialize)]
struct ProductProbe {
    dist: String,
    name: String,
    status: String,
    layout_level: Option<u16>,
    flags: Option<u16>,
    stamp: Option<u32>,
    parsed_name: Option<String>,
    name_matches_idb: bool,
    description: Option<String>,
    metadata: Vec<String>,
    image_count: Option<u32>,
    image_count_matches_idb: bool,
    images: Vec<ImageProbe>,
    failures: Vec<Failure>,
    /// Body bytes never consumed.
    leftover_bytes: usize,
}

/// Expected hierarchy of one product, from its IDB.
struct ExpectedProduct {
    name: String,
    images: Vec<ExpectedImage>,
}

struct ExpectedImage {
    short_name: String,
    subsystems: Vec<String>,
}

/// The LP16 byte pattern of a short name, for boundary checks.
fn name_pattern(short_name: &str) -> Vec<u8> {
    let bytes = latin1_bytes(short_name);
    let mut pattern = (bytes.len() as u16).to_be_bytes().to_vec();
    pattern.extend_from_slice(&bytes);
    pattern
}

fn latin1_bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u32 as u8).collect()
}

/// A possible next record: its expected name and kind.
struct BoundaryCandidate {
    pattern: Vec<u8>,
    name: String,
    is_image: bool,
    /// Matches any subsystem header with a printable name, used when
    /// the descriptor holds subsystems the IDB does not (they exist;
    /// e.g. unshipped hardware support).
    wildcard: bool,
}

/// Trial-parses a record header at `pos` without consuming: the first
/// LP16 string must equal the candidate name and every further header
/// field must stay within bounds. This keeps rule-section payloads
/// (which contain the same short names as plain blobs) from
/// masquerading as boundaries. The flag word is deliberately not
/// constrained: the corpus keeps showing new flag values.
fn valid_record_start(
    cursor: &Cursor,
    pos: usize,
    candidate: &BoundaryCandidate,
    level: u16,
) -> bool {
    let mut trial = Cursor {
        bytes: cursor.bytes,
        pos,
    };
    let parsed = (|| -> Result<(), Stop> {
        trial.be16("flags")?;
        let name = trial.lp16("name")?;
        if candidate.wildcard {
            if name.is_empty() || !name.chars().all(|c| (' '..='~').contains(&c)) {
                return Err(stop(pos, "wildcard name is not printable"));
            }
        } else if name != candidate.name {
            return Err(stop(pos, "name mismatch"));
        }
        trial.lp16("description")?;
        if candidate.is_image {
            trial.be16("field a")?;
            trial.be16("order")?;
            trial.be32("version")?;
            if level == 6 {
                trial.be32("subsystem count")?;
            } else {
                if trial.be32("reserved")? != 0 {
                    return Err(stop(pos, "reserved nonzero"));
                }
                if level == 5 {
                    trial.be16("tail unknown")?;
                }
                let metadata_count = trial.be16("metadata count")?;
                if metadata_count > MAX_SECTION_COUNT {
                    return Err(stop(pos, "metadata count"));
                }
                for _ in 0..metadata_count {
                    trial.lp16("metadata blob")?;
                }
                if level == 5 {
                    trial.be16("tail unknown")?;
                }
                trial.be16("subsystem count")?;
            }
        } else {
            trial.lp16("mapping")?;
        }
        Ok(())
    })();
    parsed.is_ok()
}

/// The index of the boundary candidate matching the current position,
/// if any.
fn at_boundary(cursor: &Cursor, candidates: &[BoundaryCandidate], level: u16) -> bool {
    candidates.iter().any(|c| {
        (c.wildcard
            || (cursor.remaining() >= 2 + c.pattern.len()
                && cursor.at(cursor.pos + 2).starts_with(&c.pattern)))
            && valid_record_start(cursor, cursor.pos, c, level)
    })
}

/// Parses the fixed product stream header and returns the layout level.
fn parse_stream_header(cursor: &mut Cursor) -> Result<u16, Stop> {
    let m1 = cursor.be16("stream magic 1")?;
    let m2 = cursor.be16("stream magic 2")?;
    let m3 = cursor.be16("stream magic 3")?;
    if (m1, m2, m3) != (0x07c4, 0x0001, 0x07c3) {
        return Err(stop(
            0,
            format!("bad stream magic: {m1:04x} {m2:04x} {m3:04x}"),
        ));
    }
    cursor.be16("layout level")
}

/// Parses one range record (`LP16, LP16, LP16, BE32 low, BE32 high`).
fn parse_range_record(cursor: &mut Cursor) -> Result<RangeRecordProbe, Stop> {
    let offset = cursor.pos;
    let a = cursor.lp16("range field 1")?;
    let b = cursor.lp16("range field 2")?;
    let c = cursor.lp16("range field 3")?;
    let low = cursor.be32("range low")?;
    let high = cursor.be32("range high")?;
    Ok(RangeRecordProbe {
        offset,
        fields: [a, b, c],
        low,
        high,
        clause_index: None,
    })
}

/// Parses a section of flat range records.
fn parse_ranges_section(
    cursor: &mut Cursor,
    offset: usize,
    count: u16,
) -> Result<SectionProbe, Stop> {
    let mut records = Vec::new();
    for _ in 0..count {
        records.push(parse_range_record(cursor)?);
    }
    Ok(SectionProbe {
        offset,
        count,
        kind: "ranges".to_string(),
        records,
        blobs: Vec::new(),
    })
}

/// Parses a clause-shaped section: `count` clauses, each a
/// `BE16 range_count + range records` group.
fn parse_clauses_section(
    cursor: &mut Cursor,
    offset: usize,
    count: u16,
) -> Result<SectionProbe, Stop> {
    let mut records = Vec::new();
    for clause_index in 0..count {
        let range_count = cursor.be16("clause range count")?;
        if range_count > MAX_SECTION_COUNT {
            return Err(stop(cursor.pos - 2, "implausible clause range count"));
        }
        for _ in 0..range_count {
            let mut record = parse_range_record(cursor)?;
            record.clause_index = Some(clause_index as u32);
            records.push(record);
        }
    }
    Ok(SectionProbe {
        offset,
        count,
        kind: "clauses".to_string(),
        records,
        blobs: Vec::new(),
    })
}

/// Parses a section of plain LP16 blobs.
fn parse_strings_section(
    cursor: &mut Cursor,
    offset: usize,
    count: u16,
) -> Result<SectionProbe, Stop> {
    let mut blobs = Vec::new();
    for _ in 0..count {
        blobs.push(cursor.lp16_max("blob", 256)?);
    }
    Ok(SectionProbe {
        offset,
        count,
        kind: "strings".to_string(),
        records: Vec::new(),
        blobs,
    })
}

/// Which record shape a rules slot holds, by index. Corpus-consistent
/// across all cleanly walked subsystems: slots 2 and 5 (and 8 on layout
/// level 9) are flat range records, slot 3 is clause-shaped, slot 7
/// (level 8/9) is a blob list, and every other slot is always zero.
fn slot_shape(slot: usize, level: u16) -> &'static str {
    match slot {
        2 | 5 => "ranges",
        3 => "clauses",
        7 if level >= 8 => "strings",
        8 if level >= 9 => "ranges",
        _ => "empty",
    }
}

/// The rules region has exactly as many slots as the layout level
/// number, each parsed in its fixed shape (see `slot_shape`).
fn parse_rules(
    cursor: &mut Cursor,
    boundary: &[BoundaryCandidate],
    level: u16,
) -> (Vec<SectionProbe>, Option<Failure>) {
    let mut sections = Vec::new();
    for slot in 0..level as usize {
        let offset = cursor.pos;
        let count = match cursor.be16("section count") {
            Ok(count) => count,
            Err(e) => {
                return (
                    sections,
                    Some(Failure {
                        offset: e.offset,
                        message: e.message,
                        context_hex: cursor.context(e.offset, 32),
                    }),
                );
            }
        };
        if count > MAX_SECTION_COUNT {
            return (
                sections,
                Some(Failure {
                    offset,
                    message: format!("slot {slot}: implausible section count {count}"),
                    context_hex: cursor.context(offset, 32),
                }),
            );
        }
        let shape = slot_shape(slot, level);
        if shape == "empty" && count != 0 {
            return (
                sections,
                Some(Failure {
                    offset,
                    message: format!("slot {slot}: expected empty, count is {count}"),
                    context_hex: cursor.context(offset, 32),
                }),
            );
        }
        let parsed = match shape {
            "ranges" => parse_ranges_section(cursor, offset, count),
            "clauses" => parse_clauses_section(cursor, offset, count),
            "strings" => parse_strings_section(cursor, offset, count),
            _ => Ok(SectionProbe {
                offset,
                count,
                kind: "empty".to_string(),
                records: Vec::new(),
                blobs: Vec::new(),
            }),
        };
        match parsed {
            Ok(section) => sections.push(section),
            Err(e) => {
                return (
                    sections,
                    Some(Failure {
                        offset: e.offset,
                        message: format!("slot {slot}: {}", e.message),
                        context_hex: cursor.context(e.offset, 32),
                    }),
                );
            }
        }
    }
    if at_boundary(cursor, boundary, level) || (boundary.is_empty() && cursor.remaining() == 0) {
        (sections, None)
    } else {
        (
            sections,
            Some(Failure {
                offset: cursor.pos,
                message: "trailing bytes after all rule slots".to_string(),
                context_hex: cursor.context(cursor.pos, 32),
            }),
        )
    }
}

/// Scans forward for the nearest expected record boundary after a walk
/// failure. Returns the resynced position.
fn resync(
    cursor: &Cursor,
    from: usize,
    candidates: &[BoundaryCandidate],
    level: u16,
) -> Option<usize> {
    let mut pos = from;
    while pos + 2 <= cursor.bytes.len() {
        for candidate in candidates {
            if (candidate.wildcard || cursor.at(pos + 2).starts_with(&candidate.pattern))
                && valid_record_start(cursor, pos, candidate, level)
            {
                return Some(pos);
            }
        }
        pos += 1;
    }
    None
}

/// Parses one descriptor body against its expected hierarchy.
fn probe_product(
    dist: &str,
    expected: &ExpectedProduct,
    descriptor_path: &std::path::Path,
) -> ProductProbe {
    let mut probe = ProductProbe {
        dist: dist.to_string(),
        name: expected.name.clone(),
        status: "fail".to_string(),
        layout_level: None,
        flags: None,
        stamp: None,
        parsed_name: None,
        name_matches_idb: false,
        description: None,
        metadata: Vec::new(),
        image_count: None,
        image_count_matches_idb: false,
        images: Vec::new(),
        failures: Vec::new(),
        leftover_bytes: 0,
    };

    let bytes = match std::fs::read(descriptor_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            probe.failures.push(Failure {
                offset: 0,
                message: format!("cannot read descriptor: {error}"),
                context_hex: String::new(),
            });
            return probe;
        }
    };
    let body = &bytes[bytes.len().min(HEADER_SIZE)..];
    let mut cursor = Cursor::new(body);

    let header = (|| -> Result<(), Stop> {
        let level = parse_stream_header(&mut cursor)?;
        probe.layout_level = Some(level);
        let name = cursor.lp16("product name")?;
        probe.name_matches_idb = name == expected.name;
        probe.parsed_name = Some(name);
        probe.description = Some(cursor.lp16("product description")?);
        probe.flags = Some(cursor.be16("product flags")?);
        probe.stamp = Some(cursor.be32("product stamp")?);
        let image_count = if level >= 8 {
            let reserved = cursor.be32("product reserved")?;
            if reserved != 0 {
                return Err(stop(
                    cursor.pos - 4,
                    format!("product reserved field is {reserved:#x}, expected 0"),
                ));
            }
            let metadata_count = cursor.be16("product metadata count")?;
            for _ in 0..metadata_count {
                probe.metadata.push(cursor.lp16("product metadata blob")?);
            }
            cursor.be16("image count")? as u32
        } else {
            cursor.be32("image count")?
        };
        probe.image_count = Some(image_count);
        probe.image_count_matches_idb = image_count as usize == expected.images.len();
        Ok(())
    })();

    if let Err(e) = header {
        probe.failures.push(Failure {
            offset: e.offset,
            message: e.message,
            context_hex: cursor.context(e.offset, 32),
        });
        return probe;
    }

    let image_count = probe.image_count.unwrap_or(0) as usize;
    let level = probe.layout_level.unwrap_or(0);
    let mut partial = !probe.name_matches_idb || !probe.image_count_matches_idb;

    // Descriptor record order does not follow IDB order, so expected
    // images and subsystems are matched by name and consumed as found.
    let mut remaining_images: Vec<&ExpectedImage> = expected.images.iter().collect();

    for image_index in 0..image_count {
        let offset = cursor.pos;
        let parsed = (|| -> Result<ImageProbe, Stop> {
            let flags = cursor.be16("image flags")?;
            let name = cursor.lp16("image name")?;
            let description = cursor.lp16("image description")?;
            let field_a = cursor.be16("image field a")?;
            let order_candidate = cursor.be16("image order candidate")?;
            let version = cursor.be32("image version")?;
            // The image tail framing depends on the layout level:
            // level 5 carries two extra unknown u16 fields around the
            // metadata blobs, level 6 is just the BE32 count, and level
            // 8/9 have metadata blobs but no extra u16 fields.
            let (tail_unknowns, metadata, subsystem_count) = if level == 6 {
                (Vec::new(), Vec::new(), cursor.be32("subsystem count")?)
            } else {
                let reserved = cursor.be32("image reserved")?;
                if reserved != 0 {
                    return Err(stop(
                        cursor.pos - 4,
                        format!("image reserved field is {reserved:#x}, expected 0"),
                    ));
                }
                let mut tail_unknowns = Vec::new();
                if level == 5 {
                    tail_unknowns.push(cursor.be16("image tail unknown 1")?);
                }
                let metadata_count = cursor.be16("image metadata count")?;
                if metadata_count > MAX_SECTION_COUNT {
                    return Err(stop(
                        cursor.pos - 2,
                        format!("implausible image metadata count {metadata_count}"),
                    ));
                }
                let mut metadata = Vec::new();
                for _ in 0..metadata_count {
                    metadata.push(cursor.lp16("image metadata blob")?);
                }
                if level == 5 {
                    tail_unknowns.push(cursor.be16("image tail unknown 2")?);
                }
                let subsystem_count = cursor.be16("subsystem count")?;
                (tail_unknowns, metadata, subsystem_count as u32)
            };
            Ok(ImageProbe {
                offset,
                expected_name: None,
                flags,
                name_matches_idb: false,
                name,
                description,
                field_a,
                order_candidate,
                version,
                tail_unknowns,
                metadata,
                subsystem_count,
                subsystems: Vec::new(),
            })
        })();

        let mut image = match parsed {
            Ok(image) => image,
            Err(e) => {
                probe.failures.push(Failure {
                    offset: e.offset,
                    message: format!("image {image_index}: {}", e.message),
                    context_hex: cursor.context(e.offset, 32),
                });
                return finish(probe, &cursor, true);
            }
        };

        let expected_image = remaining_images
            .iter()
            .position(|e| e.short_name == image.name)
            .map(|index| remaining_images.remove(index));
        image.expected_name = expected_image.map(|e| e.short_name.clone());
        image.name_matches_idb = expected_image.is_some();
        if expected_image.is_none() {
            partial = true;
        }
        if let Some(expected_image) = expected_image
            && image.subsystem_count as usize != expected_image.subsystems.len()
        {
            partial = true;
        }

        let mut remaining_subsystems: Vec<&str> = expected_image
            .map(|e| e.subsystems.iter().map(String::as_str).collect())
            .unwrap_or_default();

        for subsystem_index in 0..image.subsystem_count as usize {
            let offset = cursor.pos;
            let parsed = (|| -> Result<SubsystemProbe, Stop> {
                let flags = cursor.be16("subsystem flags")?;
                let name = cursor.lp16("subsystem name")?;
                let description = cursor.lp16("subsystem description")?;
                let mapping = cursor.lp16("subsystem mapping")?;
                Ok(SubsystemProbe {
                    offset,
                    expected_name: None,
                    flags,
                    name_matches_idb: false,
                    name,
                    description,
                    mapping,
                    sections: Vec::new(),
                    walk_stop: None,
                    unknown_tail_hex: None,
                    unknown_tail_len: 0,
                })
            })();

            let mut subsystem = match parsed {
                Ok(subsystem) => subsystem,
                Err(e) => {
                    probe.failures.push(Failure {
                        offset: e.offset,
                        message: format!(
                            "image {image_index} subsystem {subsystem_index}: {}",
                            e.message
                        ),
                        context_hex: cursor.context(e.offset, 32),
                    });
                    image.subsystems.push(SubsystemProbe {
                        offset,
                        expected_name: None,
                        flags: 0,
                        name: String::new(),
                        name_matches_idb: false,
                        description: String::new(),
                        mapping: String::new(),
                        sections: Vec::new(),
                        walk_stop: None,
                        unknown_tail_hex: None,
                        unknown_tail_len: 0,
                    });
                    probe.images.push(image);
                    return finish(probe, &cursor, true);
                }
            };

            let matched = remaining_subsystems
                .iter()
                .position(|n| *n == subsystem.name);
            subsystem.expected_name = matched.map(|i| remaining_subsystems[i].to_string());
            subsystem.name_matches_idb = matched.is_some();
            if let Some(index) = matched {
                remaining_subsystems.remove(index);
            } else {
                partial = true;
            }

            // The next boundary is any sibling subsystem not yet seen,
            // plus any image not yet seen; only one of them can validate
            // at the true boundary. The descriptor may also hold
            // subsystems the IDB does not (they exist; e.g. unshipped
            // hardware support), so while the count says more
            // subsystems are coming, any plausible subsystem header is
            // a possible next record.
            let mut boundary: Vec<BoundaryCandidate> = Vec::new();
            if subsystem_index + 1 < image.subsystem_count as usize {
                for n in &remaining_subsystems {
                    boundary.push(BoundaryCandidate {
                        pattern: name_pattern(n),
                        name: (*n).to_string(),
                        is_image: false,
                        wildcard: false,
                    });
                }
                boundary.push(BoundaryCandidate {
                    pattern: Vec::new(),
                    name: String::new(),
                    is_image: false,
                    wildcard: true,
                });
            }
            for e in &remaining_images {
                boundary.push(BoundaryCandidate {
                    pattern: name_pattern(&e.short_name),
                    name: e.short_name.clone(),
                    is_image: true,
                    wildcard: false,
                });
            }

            let (sections, walk_stop) = parse_rules(&mut cursor, &boundary, level);
            subsystem.sections = sections;
            if let Some(failure) = walk_stop {
                // The tentative walk stopped; resynchronize on the next
                // expected record boundary if there is one.
                let stop_pos = failure.offset;
                subsystem.walk_stop = Some(failure);
                partial = true;
                match resync(&cursor, stop_pos, &boundary, level) {
                    Some(pos) => {
                        subsystem.unknown_tail_len = pos - stop_pos;
                        subsystem.unknown_tail_hex =
                            Some(hex(&body[stop_pos..pos.min(stop_pos + 32)]));
                        cursor.pos = pos;
                    }
                    None => {
                        subsystem.unknown_tail_len = cursor.remaining();
                        subsystem.unknown_tail_hex = Some(cursor.context(stop_pos, 32));
                        image.subsystems.push(subsystem);
                        probe.images.push(image);
                        return finish(probe, &cursor, true);
                    }
                }
            }
            image.subsystems.push(subsystem);
        }
        probe.images.push(image);
    }

    finish(probe, &cursor, partial)
}

/// Sets the final status from exact byte consumption.
fn finish(mut probe: ProductProbe, cursor: &Cursor, mut partial: bool) -> ProductProbe {
    probe.leftover_bytes = cursor.remaining();
    if probe.leftover_bytes > 0 {
        partial = true;
    }
    if !probe.failures.is_empty() {
        partial = true;
    }
    probe.status = if partial { "partial" } else { "pass" }.to_string();
    probe
}

/// The whole probe report.
#[derive(Serialize)]
struct ProbeReport {
    tool: String,
    products: Vec<ProductProbe>,
    summary: ProbeSummary,
}

/// Corpus-wide aggregate statistics.
#[derive(Serialize)]
struct ProbeSummary {
    descriptors: usize,
    pass: usize,
    partial: usize,
    fail: usize,
    /// Status counts per layout level.
    by_level: BTreeMap<String, BTreeMap<String, usize>>,
    image_count_matches: usize,
    image_count_total: usize,
    subsystem_name_matches: usize,
    subsystem_name_total: usize,
    /// Section-count signatures per subsystem, e.g. `0,0,3,0,0`.
    section_signatures: BTreeMap<String, usize>,
    /// First character of every range record's first field.
    rule_blob_first_chars: BTreeMap<String, usize>,
    /// Walk stop messages.
    walk_stops: BTreeMap<String, usize>,
}

fn summarize(products: &[ProductProbe]) -> ProbeSummary {
    let mut summary = ProbeSummary {
        descriptors: products.len(),
        pass: 0,
        partial: 0,
        fail: 0,
        by_level: BTreeMap::new(),
        image_count_matches: 0,
        image_count_total: 0,
        subsystem_name_matches: 0,
        subsystem_name_total: 0,
        section_signatures: BTreeMap::new(),
        rule_blob_first_chars: BTreeMap::new(),
        walk_stops: BTreeMap::new(),
    };
    for product in products {
        match product.status.as_str() {
            "pass" => summary.pass += 1,
            "partial" => summary.partial += 1,
            _ => summary.fail += 1,
        }
        let level = product
            .layout_level
            .map(|l| l.to_string())
            .unwrap_or_else(|| "?".to_string());
        *summary
            .by_level
            .entry(level)
            .or_default()
            .entry(product.status.clone())
            .or_default() += 1;
        if product.image_count.is_some() {
            summary.image_count_total += 1;
            if product.image_count_matches_idb {
                summary.image_count_matches += 1;
            }
        }
        for image in &product.images {
            for subsystem in &image.subsystems {
                if subsystem.expected_name.is_some() {
                    summary.subsystem_name_total += 1;
                    if subsystem.name_matches_idb {
                        summary.subsystem_name_matches += 1;
                    }
                }
                let mut signature: Vec<String> = subsystem
                    .sections
                    .iter()
                    .map(|s| match s.kind.as_str() {
                        "clauses" => format!("{}c", s.count),
                        "strings" => format!("{}s", s.count),
                        _ => s.count.to_string(),
                    })
                    .collect();
                if subsystem.walk_stop.is_some() {
                    signature.push("STOP".to_string());
                }
                if !signature.is_empty() {
                    *summary
                        .section_signatures
                        .entry(signature.join(","))
                        .or_default() += 1;
                }
                for section in &subsystem.sections {
                    for record in &section.records {
                        let first = record.fields[0]
                            .chars()
                            .next()
                            .map(|c| c.to_string())
                            .unwrap_or_default();
                        *summary.rule_blob_first_chars.entry(first).or_default() += 1;
                    }
                }
                if let Some(failure) = &subsystem.walk_stop {
                    let key = failure
                        .message
                        .chars()
                        .take(48)
                        .collect::<String>()
                        .split_whitespace()
                        .take(3)
                        .collect::<Vec<_>>()
                        .join(" ");
                    *summary.walk_stops.entry(key).or_default() += 1;
                }
            }
        }
    }
    summary
}

fn main() -> ExitCode {
    let mut json: Option<PathBuf> = None;
    let mut dists: Vec<PathBuf> = Vec::new();
    let mut argv = std::env::args_os().skip(1);
    let arg_error = |message: &str| {
        eprintln!("usage: pd001-probe [--json <path>] <dist-dir>...");
        eprintln!("error: {message}");
        ExitCode::FAILURE
    };
    while let Some(arg) = argv.next() {
        match arg.to_str() {
            Some("--json") => match argv.next() {
                Some(path) => json = Some(PathBuf::from(path)),
                None => return arg_error("--json needs a path"),
            },
            Some(flag) if flag.starts_with('-') => {
                return arg_error(&format!("unknown option: {flag}"));
            }
            _ => dists.push(PathBuf::from(arg)),
        }
    }
    if dists.is_empty() {
        eprintln!("usage: pd001-probe [--json <path>] <dist-dir>...");
        return ExitCode::FAILURE;
    }

    let mut products = Vec::new();
    for path in &dists {
        let dist = match Distribution::open(path) {
            Ok(dist) => dist,
            Err(error) => {
                eprintln!("warning: cannot open {}: {error}", path.display());
                continue;
            }
        };
        for product in dist.products() {
            let expected = ExpectedProduct {
                name: product.name.as_str().to_string(),
                images: product
                    .images
                    .iter()
                    .map(|image| ExpectedImage {
                        short_name: image.name.image().to_string(),
                        subsystems: image
                            .subsystems
                            .iter()
                            .map(|s| s.name.subsystem().to_string())
                            .collect(),
                    })
                    .collect(),
            };
            products.push(probe_product(
                &path.display().to_string(),
                &expected,
                &product.descriptor_file,
            ));
        }
    }

    let summary = summarize(&products);
    eprintln!(
        "probed {} descriptors: {} pass, {} partial, {} fail",
        summary.descriptors, summary.pass, summary.partial, summary.fail
    );
    eprintln!(
        "image counts match IDB: {}/{}; subsystem names match IDB: {}/{}",
        summary.image_count_matches,
        summary.image_count_total,
        summary.subsystem_name_matches,
        summary.subsystem_name_total
    );
    eprintln!("section signatures:");
    for (signature, count) in &summary.section_signatures {
        eprintln!("  {count:5}  {signature}");
    }

    let report = ProbeReport {
        tool: format!("sw-core pd001-probe {}", env!("CARGO_PKG_VERSION")),
        products,
        summary,
    };
    let json_text = match serde_json::to_string_pretty(&report) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot serialize report: {error}");
            return ExitCode::FAILURE;
        }
    };
    match json {
        Some(path) => {
            if let Err(error) = std::fs::write(&path, json_text) {
                eprintln!("error: cannot write {}: {error}", path.display());
                return ExitCode::FAILURE;
            }
        }
        None => println!("{json_text}"),
    }
    ExitCode::SUCCESS
}
