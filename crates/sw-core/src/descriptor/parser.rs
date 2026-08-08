//! Parser for binary product descriptors (`pd001`).
//!
//! The grammar implemented here was established byte-by-byte against the
//! corpus of real IRIX 4.0.5 – 6.5 media (178 descriptors) and is fully
//! deterministic: every record is count-driven, nothing is searched for.
//!
//! ```text
//! file     = "pd001V..." NUL body
//! body     = 07c4 0001 07c3 layout_level product
//! integers = big-endian; strings = BE16 length + Latin-1 bytes ("LP16")
//! ```
//!
//! The framing of several regions depends on the layout level:
//!
//! * level 5/6 products carry no metadata and a BE32 image count, while
//!   level 8/9 products carry a reserved word, metadata blobs and a BE16
//!   image count;
//! * level 5 images have two extra unknown u16 fields in their tail,
//!   level 6 images have neither metadata nor reserved fields;
//! * each subsystem is followed by a rules region of exactly
//!   `layout_level` slots, each `BE16 count + records`, with a fixed
//!   shape per slot index: slots 2 and 5 (and 8 on level 9) are flat
//!   range records, slot 3 is clause-of-ranges (prerequisites), slot 7
//!   (level 8/9) is a string list, and every other slot is always zero.
use crate::descriptor::model::{
    DescriptorHeader, DescriptorImage, DescriptorMetadata, DescriptorRange, DescriptorSubsystem,
    PrerequisiteClause, ProductDescriptor, RuleSlot, Version,
};
use crate::diagnostic::Diagnostic;
use crate::error::{Error, Result};
use std::path::Path;

/// Length of the descriptor header including its trailing NUL.
pub const DESCRIPTOR_HEADER_SIZE: usize = 13;

/// Fixed stream magic at the start of the body.
const STREAM_MAGIC: [u16; 3] = [0x07c4, 0x0001, 0x07c3];

/// Layout levels observed on real media. The rules-region slot count
/// equals the level, so an unknown level cannot be framed at all.
const LAYOUT_LEVELS: [u16; 4] = [5, 6, 8, 9];

/// Sanity limit for record counts. Large core products legitimately
/// replace hundreds of legacy subsystems; nothing observed comes close
/// to this limit.
const MAX_RECORD_COUNT: u16 = 4096;

/// Big-endian cursor over the descriptor body, tracking the body offset
/// for error messages.
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

    fn be16(&mut self, path: &Path, what: &str) -> Result<u16> {
        if self.remaining() < 2 {
            return Err(Self::fail(
                path,
                self.pos,
                format!("truncated reading {what}"),
            ));
        }
        let value = u16::from_be_bytes([self.bytes[self.pos], self.bytes[self.pos + 1]]);
        self.pos += 2;
        Ok(value)
    }

    fn be32(&mut self, path: &Path, what: &str) -> Result<u32> {
        if self.remaining() < 4 {
            return Err(Self::fail(
                path,
                self.pos,
                format!("truncated reading {what}"),
            ));
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

    /// Reads a `BE16 length + bytes` blob, bounded by `max`.
    fn lp16_bytes(&mut self, path: &Path, what: &str, max: usize) -> Result<Vec<u8>> {
        let len = self.be16(path, what)? as usize;
        if len > max {
            return Err(Self::fail(
                path,
                self.pos - 2,
                format!("{what} length {len} exceeds limit {max}"),
            ));
        }
        if self.remaining() < len {
            return Err(Self::fail(
                path,
                self.pos - 2,
                format!(
                    "{what} length {len} exceeds {} remaining bytes",
                    self.remaining()
                ),
            ));
        }
        let blob = self.bytes[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(blob)
    }

    /// Reads an LP16 blob bounded only by the remaining bytes, decoded
    /// as Latin-1 (one character per byte).
    fn lp16_string(&mut self, path: &Path, what: &str) -> Result<String> {
        let bytes = self.lp16_bytes(path, what, u16::MAX as usize)?;
        Ok(bytes.iter().map(|&b| b as char).collect())
    }

    fn fail(path: &Path, offset: usize, message: String) -> Error {
        Error::Descriptor {
            path: path.to_path_buf(),
            message: format!("body offset {offset:#x}: {message}"),
        }
    }
}

/// Parses a product descriptor file.
///
/// The parse is exact: after the declared records the cursor must sit
/// precisely at the end of the file.
///
/// # Errors
///
/// Returns [`Error::Descriptor`] if the header is malformed, the stream
/// magic or layout level is unknown, a record is truncated, an
/// always-empty rule slot is non-zero, or trailing bytes remain.
pub fn parse(
    bytes: &[u8],
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<ProductDescriptor> {
    let header = parse_header(bytes, path)?;

    let mut cursor = Cursor::new(&bytes[DESCRIPTOR_HEADER_SIZE..]);
    let descriptor = parse_body(&mut cursor, header, path, diagnostics)?;
    if cursor.remaining() != 0 {
        return Err(Cursor::fail(
            path,
            cursor.pos,
            format!(
                "{} trailing bytes after the last record",
                cursor.remaining()
            ),
        ));
    }
    Ok(descriptor)
}

/// Validates and decodes the 13-byte file header.
fn parse_header(bytes: &[u8], path: &Path) -> Result<DescriptorHeader> {
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
    Ok(DescriptorHeader { magic })
}

/// Parses the body: stream header, product record and all images.
fn parse_body(
    cursor: &mut Cursor,
    header: DescriptorHeader,
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<ProductDescriptor> {
    for (index, expected) in STREAM_MAGIC.iter().enumerate() {
        let word = cursor.be16(path, "stream magic")?;
        if word != *expected {
            return Err(Cursor::fail(
                path,
                cursor.pos - 2,
                format!("bad stream magic word {index}: {word:#06x}"),
            ));
        }
    }
    let layout_level = cursor.be16(path, "layout level")?;
    if !LAYOUT_LEVELS.contains(&layout_level) {
        return Err(Cursor::fail(
            path,
            cursor.pos - 2,
            format!("unknown layout level {layout_level}"),
        ));
    }

    let name = cursor.lp16_string(path, "product name")?;
    let description = cursor.lp16_string(path, "product description")?;
    let flags_raw = cursor.be16(path, "product flags")?;
    let stamp = cursor.be32(path, "product stamp")?;

    // Level 8/9 products carry a reserved word and metadata blobs before
    // a BE16 image count; level 5/6 go straight to a BE32 image count.
    let mut metadata = Vec::new();
    let image_count = if layout_level >= 8 {
        read_reserved(cursor, path, "product", diagnostics)?;
        metadata = parse_metadata(cursor, path)?;
        u32::from(cursor.be16(path, "image count")?)
    } else {
        cursor.be32(path, "image count")?
    };

    let mut images = Vec::new();
    for _ in 0..image_count {
        images.push(parse_image(cursor, layout_level, path, diagnostics)?);
    }

    Ok(ProductDescriptor {
        header,
        layout_level,
        name,
        description,
        flags_raw,
        stamp,
        metadata,
        images,
    })
}

/// Parses one image record including its subsystems.
fn parse_image(
    cursor: &mut Cursor,
    level: u16,
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<DescriptorImage> {
    let flags_raw = cursor.be16(path, "image flags")?;
    let name = cursor.lp16_string(path, "image name")?;
    let description = cursor.lp16_string(path, "image description")?;
    let unknown_a = cursor.be16(path, "image field a")?;
    let install_order_raw = cursor.be16(path, "image order candidate")?;
    let version = Version(u64::from(cursor.be32(path, "image version")?));

    // The tail framing depends on the layout level: level 5 carries two
    // extra unknown u16 fields around the metadata blobs, level 6 is
    // just a BE32 subsystem count, and level 8/9 have metadata blobs
    // without extra u16 fields.
    let mut tail_unknowns = Vec::new();
    let mut metadata = Vec::new();
    let subsystem_count = if level == 6 {
        cursor.be32(path, "subsystem count")?
    } else {
        read_reserved(cursor, path, "image", diagnostics)?;
        if level == 5 {
            tail_unknowns.push(cursor.be16(path, "image tail unknown")?);
        }
        metadata = parse_metadata(cursor, path)?;
        if level == 5 {
            tail_unknowns.push(cursor.be16(path, "image tail unknown")?);
        }
        u32::from(cursor.be16(path, "subsystem count")?)
    };

    let mut subsystems = Vec::new();
    for _ in 0..subsystem_count {
        subsystems.push(parse_subsystem(cursor, level, path)?);
    }

    Ok(DescriptorImage {
        flags_raw,
        name,
        description,
        unknown_a,
        install_order_raw,
        version,
        tail_unknowns,
        metadata,
        subsystems,
    })
}

/// Parses one subsystem record including its rules region.
fn parse_subsystem(cursor: &mut Cursor, level: u16, path: &Path) -> Result<DescriptorSubsystem> {
    let flags_raw = cursor.be16(path, "subsystem flags")?;
    let name = cursor.lp16_string(path, "subsystem name")?;
    let description = cursor.lp16_string(path, "subsystem description")?;
    let mapping = cursor.lp16_string(path, "subsystem mapping")?;
    let (prerequisites, unassigned_slots) = parse_rules(cursor, level, path)?;
    Ok(DescriptorSubsystem {
        flags_raw,
        name,
        description,
        mapping,
        prerequisites,
        unassigned_slots,
    })
}

/// Parses the rules region: exactly `level` slots, each in the fixed
/// shape corpus-verified for its index.
fn parse_rules(
    cursor: &mut Cursor,
    level: u16,
    path: &Path,
) -> Result<(Vec<PrerequisiteClause>, Vec<RuleSlot>)> {
    let mut prerequisites = Vec::new();
    let mut unassigned = Vec::new();
    for slot in 0..level {
        let count = cursor.be16(path, "rule slot count")?;
        if count > MAX_RECORD_COUNT {
            return Err(Cursor::fail(
                path,
                cursor.pos - 2,
                format!("slot {slot}: implausible record count {count}"),
            ));
        }
        match slot_shape(slot, level) {
            SlotShape::Empty => {
                if count != 0 {
                    return Err(Cursor::fail(
                        path,
                        cursor.pos - 2,
                        format!("slot {slot}: expected empty, count is {count}"),
                    ));
                }
            }
            SlotShape::Ranges => {
                let mut values = Vec::new();
                for _ in 0..count {
                    values.push(parse_range(cursor, path)?);
                }
                if count > 0 {
                    unassigned.push(RuleSlot::Ranges {
                        slot: slot as u8,
                        values,
                    });
                }
            }
            SlotShape::Clauses => {
                for _ in 0..count {
                    let range_count = cursor.be16(path, "clause range count")?;
                    if range_count > MAX_RECORD_COUNT {
                        return Err(Cursor::fail(
                            path,
                            cursor.pos - 2,
                            format!("slot {slot}: implausible clause range count {range_count}"),
                        ));
                    }
                    let mut all_of = Vec::new();
                    for _ in 0..range_count {
                        all_of.push(parse_range(cursor, path)?);
                    }
                    prerequisites.push(PrerequisiteClause { all_of });
                }
            }
            SlotShape::Strings => {
                let mut values = Vec::new();
                for _ in 0..count {
                    // Rule strings are bounded only by the wire format
                    // (BE16 length); no smaller corpus-derived limit is
                    // imposed.
                    let blob = cursor.lp16_bytes(path, "rule string", u16::MAX as usize)?;
                    values.push(blob.iter().map(|&b| b as char).collect());
                }
                if count > 0 {
                    unassigned.push(RuleSlot::Strings {
                        slot: slot as u8,
                        values,
                    });
                }
            }
        }
    }
    Ok((prerequisites, unassigned))
}

/// The record shape of one rules-region slot, corpus-verified across
/// every subsystem of real IRIX 4.0.5 – 6.5 media.
fn slot_shape(slot: u16, level: u16) -> SlotShape {
    match slot {
        2 | 5 => SlotShape::Ranges,
        3 => SlotShape::Clauses,
        7 if level >= 8 => SlotShape::Strings,
        8 if level >= 9 => SlotShape::Ranges,
        _ => SlotShape::Empty,
    }
}

enum SlotShape {
    Empty,
    Ranges,
    Clauses,
    Strings,
}

/// Parses one range record: three LP16 fields plus BE32 low/high
/// version bounds, kept raw — real records with `low > high` exist and
/// their encoding is not yet understood.
fn parse_range(cursor: &mut Cursor, path: &Path) -> Result<DescriptorRange> {
    let product = cursor.lp16_string(path, "range product")?;
    let image = cursor.lp16_string(path, "range image")?;
    let subsystem = cursor.lp16_string(path, "range subsystem")?;
    let low = cursor.be32(path, "range low version")?;
    let high = cursor.be32(path, "range high version")?;
    Ok(DescriptorRange {
        product,
        image,
        subsystem,
        low_raw: low,
        high_raw: high,
    })
}

/// Parses a metadata blob list (`BE16 count` + LP16 blobs).
fn parse_metadata(cursor: &mut Cursor, path: &Path) -> Result<Vec<DescriptorMetadata>> {
    let count = cursor.be16(path, "metadata count")?;
    if count > MAX_RECORD_COUNT {
        return Err(Cursor::fail(
            path,
            cursor.pos - 2,
            format!("implausible metadata count {count}"),
        ));
    }
    let mut metadata = Vec::new();
    for _ in 0..count {
        let raw = cursor.lp16_bytes(path, "metadata blob", u16::MAX as usize)?;
        metadata.push(DescriptorMetadata { raw });
    }
    Ok(metadata)
}

/// Reads a reserved word that is zero on all real media. A non-zero
/// value does not change the framing, so it is a diagnostic, not an
/// error.
fn read_reserved(
    cursor: &mut Cursor,
    path: &Path,
    what: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<()> {
    let reserved = cursor.be32(path, "reserved word")?;
    if reserved != 0 {
        diagnostics.push(Diagnostic::warning(
            format!("{what} reserved word is {reserved:#x}, expected 0"),
            Some(format!("{}:#{:x}", path.display(), cursor.pos - 4)),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lp16(bytes: &mut Vec<u8>, s: &str) {
        bytes.extend_from_slice(&(s.len() as u16).to_be_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }

    fn be16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn be32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn range(bytes: &mut Vec<u8>, target: (&str, &str, &str), low: u32, high: u32) {
        lp16(bytes, target.0);
        lp16(bytes, target.1);
        lp16(bytes, target.2);
        be32(bytes, low);
        be32(bytes, high);
    }

    /// A level 9 descriptor with one image and two subsystems,
    /// exercising every non-empty slot shape.
    fn level9_descriptor() -> Vec<u8> {
        let mut bytes = b"pd001V630P00\0".to_vec();
        assert_eq!(bytes.len(), DESCRIPTOR_HEADER_SIZE);
        be16(&mut bytes, 0x07c4);
        be16(&mut bytes, 0x0001);
        be16(&mut bytes, 0x07c3);
        be16(&mut bytes, 9);
        lp16(&mut bytes, "test");
        lp16(&mut bytes, "Test Product");
        be16(&mut bytes, 0x0850);
        be32(&mut bytes, 0x0102_0304);
        be32(&mut bytes, 0); // reserved
        be16(&mut bytes, 1); // metadata count
        lp16(&mut bytes, "mCPUBOARD=IP22");
        be16(&mut bytes, 1); // image count

        be16(&mut bytes, 0x0858); // image flags
        lp16(&mut bytes, "sw");
        lp16(&mut bytes, "System Software");
        be16(&mut bytes, 0); // unknown a
        be16(&mut bytes, 9999); // order candidate
        be32(&mut bytes, 0x0102_0305); // version
        be32(&mut bytes, 0); // reserved
        be16(&mut bytes, 0); // metadata count
        be16(&mut bytes, 2); // subsystem count

        // Subsystem with one range in slot 2, one prerequisite clause
        // in slot 3 and one string in slot 7.
        be16(&mut bytes, 0x0852);
        lp16(&mut bytes, "unix");
        lp16(&mut bytes, "UNIX Kernel");
        lp16(&mut bytes, "EOE");
        be16(&mut bytes, 0); // slot 0
        be16(&mut bytes, 0); // slot 1
        be16(&mut bytes, 1); // slot 2: one range
        range(&mut bytes, ("patch*", "sw", "unix"), 0, 0x0102_0304);
        be16(&mut bytes, 1); // slot 3: one clause
        be16(&mut bytes, 1); // ... with one range
        range(&mut bytes, ("dmedia_eoe", "sw", "audio"), 0, 0x7fff_ffff);
        be16(&mut bytes, 0); // slot 4
        be16(&mut bytes, 0); // slot 5
        be16(&mut bytes, 0); // slot 6
        be16(&mut bytes, 1); // slot 7: one string
        lp16(&mut bytes, "DMODE=64bit");
        be16(&mut bytes, 0); // slot 8

        // Minimal subsystem with all slots empty.
        be16(&mut bytes, 0x0850);
        lp16(&mut bytes, "man");
        lp16(&mut bytes, "Manual Pages");
        lp16(&mut bytes, "test.sw.man");
        for _ in 0..9 {
            be16(&mut bytes, 0);
        }
        bytes
    }

    #[test]
    fn parses_level9_descriptor_exactly() {
        let bytes = level9_descriptor();
        let mut diagnostics = Vec::new();
        let descriptor = parse(&bytes, Path::new("test"), &mut diagnostics).unwrap();
        assert!(diagnostics.is_empty());

        assert_eq!(descriptor.header.magic, "pd001V630P00");
        assert_eq!(descriptor.layout_level, 9);
        assert_eq!(descriptor.name, "test");
        assert_eq!(descriptor.description, "Test Product");
        assert_eq!(descriptor.flags_raw, 0x0850);
        assert_eq!(descriptor.stamp, 0x0102_0304);
        assert_eq!(descriptor.metadata.len(), 1);
        assert!(descriptor.metadata[0].mach().unwrap().is_ok());

        assert_eq!(descriptor.images.len(), 1);
        let image = &descriptor.images[0];
        assert_eq!(image.name, "sw");
        assert_eq!(image.version, Version(0x0102_0305));
        assert_eq!(image.install_order_raw, 9999);
        assert_eq!(image.subsystems.len(), 2);

        let unix = &image.subsystems[0];
        assert_eq!(unix.name, "unix");
        assert_eq!(unix.mapping, "EOE");
        assert_eq!(unix.prerequisites.len(), 1);
        let clause_range = &unix.prerequisites[0].all_of[0];
        assert_eq!(clause_range.target(), "dmedia_eoe.sw.audio");
        assert!(clause_range.high_is_maxint());
        assert!(clause_range.ordinary_version_range().is_some());

        let slot2 = unix
            .unassigned_slots
            .iter()
            .find(|s| s.slot() == 2)
            .expect("slot 2");
        let RuleSlot::Ranges { values, .. } = slot2 else {
            panic!("slot 2 must be ranges");
        };
        assert_eq!(values[0].target(), "patch*.sw.unix");
        assert_eq!(values[0].low_raw, 0);
        assert_eq!(values[0].high_raw, 0x0102_0304);

        let slot7 = unix
            .unassigned_slots
            .iter()
            .find(|s| s.slot() == 7)
            .expect("slot 7");
        let RuleSlot::Strings { values, .. } = slot7 else {
            panic!("slot 7 must be strings");
        };
        assert_eq!(values, &["DMODE=64bit".to_string()]);

        let man = &image.subsystems[1];
        assert!(man.prerequisites.is_empty());
        assert!(man.unassigned_slots.is_empty());
    }

    #[test]
    fn parses_level5_and_level6_framing() {
        for level in [5u16, 6] {
            let mut bytes = b"pd001V530P00\0".to_vec();
            be16(&mut bytes, 0x07c4);
            be16(&mut bytes, 0x0001);
            be16(&mut bytes, 0x07c3);
            be16(&mut bytes, level);
            lp16(&mut bytes, "old");
            lp16(&mut bytes, "Old Product");
            be16(&mut bytes, 0x0050);
            be32(&mut bytes, 7);
            be32(&mut bytes, 1); // image count (BE32 on level 5/6)

            be16(&mut bytes, 0x0058);
            lp16(&mut bytes, "sw");
            lp16(&mut bytes, "System Software");
            be16(&mut bytes, 0);
            be16(&mut bytes, 600);
            be32(&mut bytes, 42);
            if level == 6 {
                be32(&mut bytes, 1); // subsystem count (BE32, no metadata)
            } else {
                be32(&mut bytes, 0); // reserved
                be16(&mut bytes, 0x1111); // tail unknown 1
                be16(&mut bytes, 0); // metadata count
                be16(&mut bytes, 0x2222); // tail unknown 2
                be16(&mut bytes, 1); // subsystem count
            }

            be16(&mut bytes, 0x0052);
            lp16(&mut bytes, "base");
            lp16(&mut bytes, "Base");
            lp16(&mut bytes, "old.sw.base");
            for _ in 0..level {
                be16(&mut bytes, 0);
            }

            let mut diagnostics = Vec::new();
            let descriptor = parse(&bytes, Path::new("old"), &mut diagnostics).unwrap();
            assert!(diagnostics.is_empty());
            assert_eq!(descriptor.layout_level, level);
            let image = &descriptor.images[0];
            assert_eq!(image.version, Version(42));
            if level == 5 {
                assert_eq!(image.tail_unknowns, vec![0x1111, 0x2222]);
            } else {
                assert!(image.tail_unknowns.is_empty());
                assert!(image.metadata.is_empty());
            }
            assert_eq!(image.subsystems[0].name, "base");
        }
    }

    #[test]
    fn rejects_malformed_bodies() {
        let good = level9_descriptor();
        let mut diagnostics = Vec::new();

        // Bad stream magic.
        let mut bad = good.clone();
        bad[DESCRIPTOR_HEADER_SIZE] = 0xff;
        assert!(parse(&bad, Path::new("test"), &mut diagnostics).is_err());

        // Unknown layout level.
        let mut bad = good.clone();
        let level_at = DESCRIPTOR_HEADER_SIZE + 7;
        bad[level_at] = 7;
        assert!(parse(&bad, Path::new("test"), &mut diagnostics).is_err());

        // Trailing bytes.
        let mut bad = good.clone();
        bad.push(0);
        assert!(parse(&bad, Path::new("test"), &mut diagnostics).is_err());

        // Truncation anywhere must fail, not panic.
        for cut in 0..good.len() {
            assert!(
                parse(&good[..cut], Path::new("test"), &mut diagnostics).is_err(),
                "truncated at {cut} bytes must fail"
            );
        }
    }

    #[test]
    fn rejects_non_zero_count_in_always_empty_slot() {
        let mut bytes = level9_descriptor();
        // First slot-0 count of subsystem `unix`: walk to it. Header 13,
        // stream 8, product 2+4 +2+12 +2+4, reserved 4, meta 2 + 2+13,
        // image count 2, image flags 2, name 2+2, desc 2+15, fields
        // 2+2+4, reserved 4, meta 2, count 2, subsystem flags 2,
        // name 2+4, desc 2+11, mapping 2+3, then slot 0 count.
        let slot0 = 13
            + 8
            + (2 + 4)
            + (2 + 12)
            + 2
            + 4
            + 4
            + (2 + 2 + 13)
            + 2
            + 2
            + (2 + 2)
            + (2 + 15)
            + 2
            + 2
            + 4
            + 4
            + 2
            + 2
            + 2
            + (2 + 4)
            + (2 + 11)
            + (2 + 3);
        bytes[slot0] = 0;
        bytes[slot0 + 1] = 1;
        let mut diagnostics = Vec::new();
        let error = parse(&bytes, Path::new("test"), &mut diagnostics).unwrap_err();
        assert!(error.to_string().contains("slot 0"), "{error}");
    }

    #[test]
    fn rejects_bad_input() {
        let mut diagnostics = Vec::new();
        assert!(parse(b"short", Path::new("x"), &mut diagnostics).is_err());
        assert!(parse(b"im001V530P00\0x", Path::new("x"), &mut diagnostics).is_err());
        assert!(parse(b"pd001V530P00Xx", Path::new("x"), &mut diagnostics).is_err());
    }
}
