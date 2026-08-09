//! Parser for binary product descriptors (`pd001`).
//!
//! The grammar implemented here mirrors the `gendist` descriptor writer
//! field by field and is fully deterministic: every record is
//! count-driven, nothing is searched for.
//!
//! ```text
//! file     = "pd001V..." NUL body
//! body     = 07c4 0001 07c3 layout_level product
//! integers = big-endian; strings = BE16 length + Latin-1 bytes ("LP16")
//! ```
//!
//! Records (fields guarded by a level are only present from that
//! layout level on):
//!
//! ```text
//! product   = LP16 name, LP16 id, BE16 flags, BE32 stamp,
//!             [L5] LP16 desc,
//!             [L7] attributes,
//!             BE16 image_count, images
//! image     = BE16 flags, LP16 name, LP16 id,
//!             BE16 legacy_field, BE16 order, BE32 version,
//!             [L5 only] BE32 reserved, BE32 reserved,
//!             [L5] LP16 desc,
//!             [L8] attributes,
//!             BE16 subsystem_count, subsystems
//! subsystem = BE16 flags, LP16 name, LP16 id, LP16 mapping,
//!             BE32 legacy_ordinal,
//!             BE16 count + ranges            (replaces/obsoletes/follows)
//!             BE16 count + clauses           (prereq: BE16 count + ranges)
//!             [L5] LP16 desc,
//!             [L6] BE16 count + ranges       (incompat)
//!             [L8] attributes
//!             [L9] BE16 count + ranges       (updates)
//! attributes = BE32 count + LP16 blobs (one tag byte + payload)
//! ```
//!
//! The string length written by `gendist` is a *signed* 16-bit value, so
//! no legitimate string exceeds 32767 bytes.
use crate::descriptor::model::{
    DescriptorAttribute, DescriptorHeader, DescriptorImage, DescriptorRange, DescriptorSubsystem,
    DescriptorText, ProductDescriptor, Version,
};
use crate::diagnostic::Diagnostic;
use crate::error::{Error, Result};
use std::path::Path;

/// Length of the descriptor header including its trailing NUL.
pub const DESCRIPTOR_HEADER_SIZE: usize = 13;

/// Fixed stream magic at the start of the body.
const STREAM_MAGIC: [u16; 3] = [0x07c4, 0x0001, 0x07c3];

/// Layout levels the reader accepts. The level is the minimum format
/// version the product's features required; level 7 exists even though
/// no observed medium carries it.
const MIN_LAYOUT_LEVEL: u16 = 5;
/// Highest layout level the writer produces.
const MAX_LAYOUT_LEVEL: u16 = 9;

/// Maximum byte length of a length-prefixed string. The writer stores
/// the length in a signed 16-bit value, so this is a format limit, not
/// a corpus-derived sanity bound.
const MAX_STRING_LENGTH: usize = 32767;

/// Minimum wire size of one range record: three empty LP16 strings and
/// two BE32 bounds.
const MIN_RANGE_SIZE: usize = 3 * 2 + 2 * 4;

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

    /// Reads a `BE16 length + bytes` blob.
    fn lp16_bytes(&mut self, path: &Path, what: &str) -> Result<Vec<u8>> {
        let len = self.be16(path, what)? as usize;
        if len > MAX_STRING_LENGTH {
            return Err(Self::fail(
                path,
                self.pos - 2,
                format!("{what} length {len} exceeds the wire maximum {MAX_STRING_LENGTH}"),
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

    /// Reads an LP16 blob decoded as Latin-1 (one character per byte).
    fn lp16_string(&mut self, path: &Path, what: &str) -> Result<String> {
        let bytes = self.lp16_bytes(path, what)?;
        Ok(bytes.iter().map(|&b| b as char).collect())
    }

    fn fail(path: &Path, offset: usize, message: String) -> Error {
        Error::Descriptor {
            path: path.to_path_buf(),
            message: format!("body offset {offset:#x}: {message}"),
        }
    }

    /// Rejects a record count that cannot possibly fit in the remaining
    /// bytes, given each record's minimum wire size. This derives the
    /// limit from the data itself instead of imposing a ceiling the
    /// format does not have.
    fn check_count(&self, path: &Path, count: u64, min_each: usize, what: &str) -> Result<()> {
        if count.saturating_mul(min_each as u64) > self.remaining() as u64 {
            return Err(Self::fail(
                path,
                self.pos,
                format!(
                    "{what}: {count} records of at least {min_each} bytes each exceed \
                     {} remaining bytes",
                    self.remaining()
                ),
            ));
        }
        Ok(())
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
/// magic or layout level is unknown, a record is truncated or
/// over-long, or trailing bytes remain.
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
    let generation_id = String::from_utf8_lossy(&bytes[..DESCRIPTOR_HEADER_SIZE - 1]).into_owned();
    Ok(DescriptorHeader { generation_id })
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
    if !(MIN_LAYOUT_LEVEL..=MAX_LAYOUT_LEVEL).contains(&layout_level) {
        return Err(Cursor::fail(
            path,
            cursor.pos - 2,
            format!("unknown layout level {layout_level}"),
        ));
    }

    let name = cursor.lp16_string(path, "product name")?;
    let id = cursor.lp16_string(path, "product id")?;
    let flags_raw = cursor.be16(path, "product flags")?;
    let stamp = cursor.be32(path, "product stamp")?;
    let description = cursor.lp16_string(path, "product desc")?;
    let attributes = if layout_level >= 7 {
        parse_attributes(cursor, path)?
    } else {
        Vec::new()
    };

    let image_count = cursor.be16(path, "image count")?;
    let mut images = Vec::new();
    for _ in 0..image_count {
        images.push(parse_image(cursor, layout_level, path, diagnostics)?);
    }

    Ok(ProductDescriptor {
        header,
        layout_level,
        name,
        text: DescriptorText { id, description },
        flags_raw,
        stamp,
        attributes,
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
    let id = cursor.lp16_string(path, "image id")?;
    let legacy_field = cursor.be16(path, "image legacy field")?;
    let order = cursor.be16(path, "image order")?;
    let version = Version(u64::from(cursor.be32(path, "image version")?));
    if level == 5 {
        read_reserved(cursor, path, "image reserved word", diagnostics)?;
        read_reserved(cursor, path, "image reserved word", diagnostics)?;
    }
    let description = cursor.lp16_string(path, "image desc")?;
    let attributes = if level >= 8 {
        parse_attributes(cursor, path)?
    } else {
        Vec::new()
    };

    let subsystem_count = cursor.be16(path, "subsystem count")?;
    let mut subsystems = Vec::new();
    for _ in 0..subsystem_count {
        subsystems.push(parse_subsystem(cursor, level, path)?);
    }

    Ok(DescriptorImage {
        flags_raw,
        name,
        text: DescriptorText { id, description },
        legacy_field,
        order,
        version,
        attributes,
        subsystems,
    })
}

/// Parses one subsystem record including its rules region.
fn parse_subsystem(cursor: &mut Cursor, level: u16, path: &Path) -> Result<DescriptorSubsystem> {
    let flags_raw = cursor.be16(path, "subsystem flags")?;
    let name = cursor.lp16_string(path, "subsystem name")?;
    let id = cursor.lp16_string(path, "subsystem id")?;
    let mapping = cursor.lp16_string(path, "subsystem mapping")?;
    let legacy_ordinal = cursor.be32(path, "subsystem legacy ordinal")?;

    let replaces = parse_ranges(cursor, path, "replaces")?;
    let prerequisites = parse_clauses(cursor, path)?;
    let description = cursor.lp16_string(path, "subsystem desc")?;
    let incompatibilities = if level >= 6 {
        parse_ranges(cursor, path, "incompat")?
    } else {
        Vec::new()
    };
    let attributes = if level >= 8 {
        parse_attributes(cursor, path)?
    } else {
        Vec::new()
    };
    let updates = if level >= 9 {
        parse_ranges(cursor, path, "updates")?
    } else {
        Vec::new()
    };

    Ok(DescriptorSubsystem {
        flags_raw,
        name,
        text: DescriptorText { id, description },
        mapping,
        legacy_ordinal,
        replaces,
        prerequisites,
        incompatibilities,
        attributes,
        updates,
    })
}

/// Parses a `BE16 count + range records` list.
fn parse_ranges(cursor: &mut Cursor, path: &Path, what: &str) -> Result<Vec<DescriptorRange>> {
    let count = cursor.be16(path, "range count")?;
    cursor.check_count(path, u64::from(count), MIN_RANGE_SIZE, what)?;
    let mut ranges = Vec::new();
    for _ in 0..count {
        ranges.push(parse_range(cursor, path)?);
    }
    Ok(ranges)
}

/// Parses prerequisite clauses: a `BE16` clause count, then per clause
/// a `BE16` range count and that many range records.
fn parse_clauses(cursor: &mut Cursor, path: &Path) -> Result<Vec<Vec<DescriptorRange>>> {
    let count = cursor.be16(path, "prerequisite clause count")?;
    cursor.check_count(path, u64::from(count), 2, "prerequisite clauses")?;
    let mut clauses = Vec::new();
    for _ in 0..count {
        let range_count = cursor.be16(path, "clause range count")?;
        cursor.check_count(
            path,
            u64::from(range_count),
            MIN_RANGE_SIZE,
            "clause ranges",
        )?;
        let mut clause = Vec::new();
        for _ in 0..range_count {
            clause.push(parse_range(cursor, path)?);
        }
        clauses.push(clause);
    }
    Ok(clauses)
}

/// Parses one range record: three LP16 fields plus BE32 low/high
/// version bounds, kept raw — `follows` records carry a negated low
/// bound (see [`DescriptorRange::is_follows`]).
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

/// Parses an attribute list: a `BE32` count followed by that many LP16
/// blobs, each split into its tag byte and payload text.
fn parse_attributes(cursor: &mut Cursor, path: &Path) -> Result<Vec<DescriptorAttribute>> {
    let count = cursor.be32(path, "attribute count")?;
    cursor.check_count(path, u64::from(count), 2, "attributes")?;
    let mut attributes = Vec::new();
    for _ in 0..count {
        let blob = cursor.lp16_bytes(path, "attribute blob")?;
        let (&tag, text) = blob
            .split_first()
            .ok_or_else(|| Cursor::fail(path, cursor.pos, "empty attribute blob".to_string()))?;
        attributes.push(DescriptorAttribute {
            tag,
            text: text.iter().map(|&b| b as char).collect(),
        });
    }
    Ok(attributes)
}

/// Reads a reserved BE32 word that is zero on all real media. A
/// non-zero value does not change the framing, so it is a diagnostic,
/// not an error.
fn read_reserved(
    cursor: &mut Cursor,
    path: &Path,
    what: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<()> {
    let reserved = cursor.be32(path, "reserved word")?;
    if reserved != 0 {
        diagnostics.push(Diagnostic::warning(
            format!("{what} is {reserved:#x}, expected 0"),
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

    fn stream_header(bytes: &mut Vec<u8>, level: u16) {
        be16(bytes, 0x07c4);
        be16(bytes, 0x0001);
        be16(bytes, 0x07c3);
        be16(bytes, level);
    }

    /// A level 9 descriptor with one image and two subsystems,
    /// exercising every present field.
    fn level9_descriptor() -> Vec<u8> {
        let mut bytes = b"pd001V630P00\0".to_vec();
        assert_eq!(bytes.len(), DESCRIPTOR_HEADER_SIZE);
        stream_header(&mut bytes, 9);
        lp16(&mut bytes, "test");
        lp16(&mut bytes, "Test Product");
        be16(&mut bytes, 0x0850);
        be32(&mut bytes, 0x0102_0304);
        lp16(&mut bytes, ""); // desc
        be32(&mut bytes, 1); // attribute count
        lp16(&mut bytes, "mCPUBOARD=IP22");
        be16(&mut bytes, 1); // image count

        be16(&mut bytes, 0x0858); // image flags
        lp16(&mut bytes, "sw");
        lp16(&mut bytes, "System Software");
        be16(&mut bytes, 2); // legacy field
        be16(&mut bytes, 9999); // order
        be32(&mut bytes, 0x0102_0305); // version
        lp16(&mut bytes, ""); // desc
        be32(&mut bytes, 0); // attribute count
        be16(&mut bytes, 2); // subsystem count

        // Subsystem with one replaces range, one prerequisite clause
        // and one attribute.
        be16(&mut bytes, 0x0852);
        lp16(&mut bytes, "unix");
        lp16(&mut bytes, "UNIX Kernel");
        lp16(&mut bytes, "EOE");
        be32(&mut bytes, 0); // legacy ordinal
        be16(&mut bytes, 1); // replaces count
        range(&mut bytes, ("patch*", "sw", "unix"), 0, 0x0102_0304);
        be16(&mut bytes, 1); // prerequisite clause count
        be16(&mut bytes, 1); // ... with one range
        range(&mut bytes, ("dmedia_eoe", "sw", "audio"), 0, 0x7fff_ffff);
        lp16(&mut bytes, ""); // desc
        be16(&mut bytes, 0); // incompat count
        be32(&mut bytes, 1); // attribute count
        lp16(&mut bytes, "DMODE=64bit");
        be16(&mut bytes, 0); // updates count

        // Minimal subsystem with everything empty.
        be16(&mut bytes, 0x0850);
        lp16(&mut bytes, "man");
        lp16(&mut bytes, "Manual Pages");
        lp16(&mut bytes, "test.sw.man");
        be32(&mut bytes, 0); // legacy ordinal
        be16(&mut bytes, 0); // replaces
        be16(&mut bytes, 0); // prerequisites
        lp16(&mut bytes, ""); // desc
        be16(&mut bytes, 0); // incompat
        be32(&mut bytes, 0); // attributes
        be16(&mut bytes, 0); // updates
        bytes
    }

    #[test]
    fn parses_level9_descriptor_exactly() {
        let bytes = level9_descriptor();
        let mut diagnostics = Vec::new();
        let descriptor = parse(&bytes, Path::new("test"), &mut diagnostics).unwrap();
        assert!(diagnostics.is_empty());

        assert_eq!(descriptor.header.generation_id, "pd001V630P00");
        assert_eq!(descriptor.layout_level, 9);
        assert_eq!(descriptor.name, "test");
        assert_eq!(descriptor.text.id, "Test Product");
        assert_eq!(descriptor.text.effective(), "Test Product");
        assert_eq!(descriptor.flags_raw, 0x0850);
        assert_eq!(descriptor.stamp, 0x0102_0304);
        assert_eq!(descriptor.attributes.len(), 1);
        assert!(descriptor.attributes[0].mach().unwrap().is_ok());

        assert_eq!(descriptor.images.len(), 1);
        let image = &descriptor.images[0];
        assert_eq!(image.name, "sw");
        assert_eq!(image.version, Version(0x0102_0305));
        assert_eq!(image.order, 9999);
        assert_eq!(image.legacy_field, 2);
        assert_eq!(image.subsystems.len(), 2);

        let unix = &image.subsystems[0];
        assert_eq!(unix.name, "unix");
        assert_eq!(unix.mapping, "EOE");
        assert_eq!(unix.replaces.len(), 1);
        assert_eq!(unix.replaces[0].target(), "patch*.sw.unix");
        assert_eq!(unix.replaces[0].low_raw, 0);
        assert_eq!(unix.replaces[0].high_raw, 0x0102_0304);
        assert!(unix.replaces[0].ordinary_version_range().is_some());

        assert_eq!(unix.prerequisites.len(), 1);
        let clause_range = &unix.prerequisites[0][0];
        assert_eq!(clause_range.target(), "dmedia_eoe.sw.audio");
        assert!(clause_range.high_is_maxint());

        assert_eq!(unix.attributes.len(), 1);
        assert_eq!(unix.attributes[0].tag, b'D');
        assert_eq!(unix.attributes[0].text, "MODE=64bit");

        let man = &image.subsystems[1];
        assert!(man.replaces.is_empty());
        assert!(man.prerequisites.is_empty());
        assert!(man.incompatibilities.is_empty());
        assert!(man.attributes.is_empty());
        assert!(man.updates.is_empty());
    }

    #[test]
    fn parses_level5_and_level6_framing() {
        for level in [5u16, 6] {
            let mut bytes = b"pd001V530P00\0".to_vec();
            stream_header(&mut bytes, level);
            lp16(&mut bytes, "old");
            lp16(&mut bytes, "Old Product");
            be16(&mut bytes, 0x0050);
            be32(&mut bytes, 7);
            lp16(&mut bytes, ""); // desc
            be16(&mut bytes, 1); // image count

            be16(&mut bytes, 0x0058);
            lp16(&mut bytes, "sw");
            lp16(&mut bytes, "System Software");
            be16(&mut bytes, 2); // legacy field
            be16(&mut bytes, 600); // order
            be32(&mut bytes, 42); // version
            if level == 5 {
                be32(&mut bytes, 0); // reserved
                be32(&mut bytes, 0); // reserved
            }
            lp16(&mut bytes, ""); // desc
            be16(&mut bytes, 1); // subsystem count

            be16(&mut bytes, 0x0052);
            lp16(&mut bytes, "base");
            lp16(&mut bytes, "Base");
            lp16(&mut bytes, "old.sw.base");
            be32(&mut bytes, 0); // legacy ordinal
            be16(&mut bytes, 0); // replaces
            be16(&mut bytes, 0); // prerequisites
            lp16(&mut bytes, ""); // desc
            if level >= 6 {
                be16(&mut bytes, 0); // incompat
            }

            let mut diagnostics = Vec::new();
            let descriptor = parse(&bytes, Path::new("old"), &mut diagnostics).unwrap();
            assert!(diagnostics.is_empty());
            assert_eq!(descriptor.layout_level, level);
            assert!(descriptor.attributes.is_empty());
            let image = &descriptor.images[0];
            assert_eq!(image.version, Version(42));
            assert_eq!(image.order, 600);
            assert!(image.attributes.is_empty());
            let subsystem = &image.subsystems[0];
            assert_eq!(subsystem.name, "base");
            assert!(subsystem.incompatibilities.is_empty());
            assert!(subsystem.attributes.is_empty());
        }
    }

    #[test]
    fn parses_level7_product_attributes() {
        let mut bytes = b"pd001V620P02\0".to_vec();
        stream_header(&mut bytes, 7);
        lp16(&mut bytes, "mid");
        lp16(&mut bytes, "Mid Product");
        be16(&mut bytes, 0x0050);
        be32(&mut bytes, 9);
        lp16(&mut bytes, ""); // desc
        be32(&mut bytes, 1); // attribute count (mach only at level 7)
        lp16(&mut bytes, "mCPUBOARD=IP22");
        be16(&mut bytes, 1); // image count

        be16(&mut bytes, 0x0058);
        lp16(&mut bytes, "sw");
        lp16(&mut bytes, "System Software");
        be16(&mut bytes, 2);
        be16(&mut bytes, 900);
        be32(&mut bytes, 100);
        lp16(&mut bytes, ""); // desc
        be16(&mut bytes, 1); // subsystem count

        be16(&mut bytes, 0x0052);
        lp16(&mut bytes, "base");
        lp16(&mut bytes, "Base");
        lp16(&mut bytes, "mid.sw.base");
        be32(&mut bytes, 0);
        be16(&mut bytes, 0);
        be16(&mut bytes, 0);
        lp16(&mut bytes, ""); // desc
        be16(&mut bytes, 0); // incompat

        let mut diagnostics = Vec::new();
        let descriptor = parse(&bytes, Path::new("mid"), &mut diagnostics).unwrap();
        assert!(diagnostics.is_empty());
        assert_eq!(descriptor.layout_level, 7);
        assert_eq!(descriptor.attributes.len(), 1);
        assert!(descriptor.attributes[0].mach().unwrap().is_ok());
        // Level 7 images and subsystems carry no attributes.
        assert!(descriptor.images[0].attributes.is_empty());
        assert!(descriptor.images[0].subsystems[0].attributes.is_empty());
    }

    #[test]
    fn reports_non_zero_reserved_words_as_diagnostics() {
        let mut bytes = b"pd001V530P00\0".to_vec();
        stream_header(&mut bytes, 5);
        lp16(&mut bytes, "old");
        lp16(&mut bytes, "Old Product");
        be16(&mut bytes, 0x0050);
        be32(&mut bytes, 7);
        lp16(&mut bytes, "");
        be16(&mut bytes, 1); // image count

        be16(&mut bytes, 0x0058);
        lp16(&mut bytes, "sw");
        lp16(&mut bytes, "System Software");
        be16(&mut bytes, 2);
        be16(&mut bytes, 600);
        be32(&mut bytes, 42);
        be32(&mut bytes, 0xdead_beef); // reserved, non-zero
        be32(&mut bytes, 0); // reserved
        lp16(&mut bytes, "");
        be16(&mut bytes, 1); // subsystem count

        be16(&mut bytes, 0x0052);
        lp16(&mut bytes, "base");
        lp16(&mut bytes, "Base");
        lp16(&mut bytes, "old.sw.base");
        be32(&mut bytes, 7); // legacy ordinal: a genuine wire field, kept
        be16(&mut bytes, 0);
        be16(&mut bytes, 0);
        lp16(&mut bytes, "");

        let mut diagnostics = Vec::new();
        let descriptor = parse(&bytes, Path::new("old"), &mut diagnostics).unwrap();
        let subsystem = &descriptor.images[0].subsystems[0];
        assert_eq!(subsystem.name, "base");
        assert_eq!(subsystem.legacy_ordinal, 7);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("reserved"));
    }

    #[test]
    fn rejects_malformed_bodies() {
        let good = level9_descriptor();
        let mut diagnostics = Vec::new();

        // Bad stream magic.
        let mut bad = good.clone();
        bad[DESCRIPTOR_HEADER_SIZE] = 0xff;
        assert!(parse(&bad, Path::new("test"), &mut diagnostics).is_err());

        // Layout levels outside 5..=9 are unknown.
        for level in [4u8, 10] {
            let mut bad = good.clone();
            let level_at = DESCRIPTOR_HEADER_SIZE + 7;
            bad[level_at] = level;
            assert!(parse(&bad, Path::new("test"), &mut diagnostics).is_err());
        }

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
    fn rejects_overlong_strings() {
        let mut bytes = b"pd001V630P00\0".to_vec();
        stream_header(&mut bytes, 9);
        // The wire length is a signed 16-bit value; 32768 is impossible.
        bytes.extend_from_slice(&32768u16.to_be_bytes());
        let mut diagnostics = Vec::new();
        let error = parse(&bytes, Path::new("test"), &mut diagnostics).unwrap_err();
        assert!(error.to_string().contains("wire maximum"), "{error}");
    }

    #[test]
    fn rejects_bad_input() {
        let mut diagnostics = Vec::new();
        assert!(parse(b"short", Path::new("x"), &mut diagnostics).is_err());
        assert!(parse(b"im001V530P00\0x", Path::new("x"), &mut diagnostics).is_err());
        assert!(parse(b"pd001V530P00Xx", Path::new("x"), &mut diagnostics).is_err());
    }
}
