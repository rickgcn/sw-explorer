//! Decodes physical descriptor records into logical semantics.
//!
//! The attribute tags and flag bits interpreted here are the ones the
//! descriptor writer produces:
//!
//! * attribute tags `m`, `R`, `D`, `M` carry hardware expressions
//!   (`mach` and conditional `required`/`default`/`miniroot`);
//! * tag `A` carries an `autominiroot` rule as `name low high`;
//! * tag `C` carries a product cut point as `path`, a `0x01` separator
//!   and a sequence number;
//! * the subsystem flag word carries `required` (`0x0001`), `default`
//!   (`0x0002`), `patch` (`0x0400`), `inplace` (`0x0800`, cleared for
//!   miniroot subsystems), `clientonly` (`0x2000`), overlay member
//!   (`0x4000`) and `overlays` (`0x8000`).
//!
//! A conditional flag's bit is set whether or not the spec statement
//! had a hardware expression, so the blobs decide between
//! [`ConditionalFlag::Always`] and [`ConditionalFlag::When`].
//!
//! Everything here is fail-safe: an expression that cannot be parsed is
//! preserved as *unresolved* so evaluation can report "unknown" instead
//! of silently treating the restriction as absent.
use crate::descriptor::model::{
    ConditionalFlag, Cutpoint, DescriptorAttribute, DescriptorRange, DescriptorSubsystem,
    HardwareRestrictions, PrerequisiteClause, SubsystemFlags, SubsystemRange, Version,
    VersionRange, version_limit,
};
use crate::diagnostic::Diagnostic;
use crate::mach::HardwareExpr;
use crate::path::IrixPath;

/// Subsystem flag bit: `required`.
const FLAG_REQUIRED: u16 = 0x0001;
/// Subsystem flag bit: `default`.
const FLAG_DEFAULT: u16 = 0x0002;
/// Subsystem flag bit: `patch`.
const FLAG_PATCH: u16 = 0x0400;
/// Subsystem flag bit: inplace; cleared for miniroot subsystems.
const FLAG_INPLACE: u16 = 0x0800;
/// Subsystem flag bit: `clientonly`.
const FLAG_CLIENT_ONLY: u16 = 0x2000;
/// Subsystem flag bit: member of an overlay product.
const FLAG_OVERLAY_MEMBER: u16 = 0x4000;
/// Subsystem flag bit: `overlays` (overlay subsystem).
const FLAG_OVERLAY: u16 = 0x8000;

/// Collects the hardware expressions of every `mach` attribute.
/// Unparsable expressions are reported as warnings *and* preserved in
/// the result's `unresolved` list, so applicability stays fail-safe.
pub(crate) fn mach_expressions(
    attributes: &[DescriptorAttribute],
    origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> HardwareRestrictions {
    let (expressions, unresolved) =
        tagged_expressions(attributes, b'm', "mach", origin, diagnostics);
    HardwareRestrictions {
        expressions,
        unresolved,
    }
}

/// Decodes the installation flags of a subsystem from its raw flag word
/// and its conditional flag attributes.
pub(crate) fn decode_flags(
    flags_raw: u16,
    attributes: &[DescriptorAttribute],
    origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> SubsystemFlags {
    let (required, required_bad) =
        tagged_expressions(attributes, b'R', "required", origin, diagnostics);
    let (default, default_bad) =
        tagged_expressions(attributes, b'D', "default", origin, diagnostics);
    let (miniroot, miniroot_bad) =
        tagged_expressions(attributes, b'M', "miniroot", origin, diagnostics);

    let inplace = flags_raw & FLAG_INPLACE != 0;
    SubsystemFlags {
        required: conditional_flag(flags_raw & FLAG_REQUIRED != 0, required, required_bad),
        default: conditional_flag(flags_raw & FLAG_DEFAULT != 0, default, default_bad),
        // The wire default is "inplace"; `miniroot` clears the bit.
        miniroot: conditional_flag(!inplace, miniroot, miniroot_bad),
        inplace,
        patch: flags_raw & FLAG_PATCH != 0,
        client_only: flags_raw & FLAG_CLIENT_ONLY != 0,
        overlay: flags_raw & FLAG_OVERLAY != 0,
        overlay_member: flags_raw & FLAG_OVERLAY_MEMBER != 0,
    }
}

/// Decodes the dependency rules of a subsystem record.
///
/// The `replaces` list is split into `follows` records (negative low
/// bound encoding) and ordinary `replaces`/`obsoletes` ranges.
pub(crate) fn decode_rules(
    record: &DescriptorSubsystem,
    origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> crate::descriptor::model::SubsystemRules {
    let mut rules = crate::descriptor::model::SubsystemRules {
        prerequisites: decode_prerequisites(&record.prerequisites, origin, diagnostics),
        ..Default::default()
    };
    for range in &record.replaces {
        if let Some(versions) = range.follows_version_range() {
            rules.follows.push(range.to_logical(versions));
        } else if let Some(versions) = decode_ordinary(range, origin, diagnostics) {
            rules.replaces.push(range.to_logical(versions));
        }
    }
    for range in &record.incompatibilities {
        if let Some(versions) = decode_ordinary(range, origin, diagnostics) {
            rules.incompatibilities.push(range.to_logical(versions));
        }
    }
    for range in &record.updates {
        if let Some(versions) = decode_ordinary(range, origin, diagnostics) {
            rules.updates.push(range.to_logical(versions));
        }
    }
    rules
}

/// Decodes the `autominiroot` attributes (`A` blobs,
/// `name low high`) into ranges.
pub(crate) fn autominiroot(
    attributes: &[DescriptorAttribute],
    origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<SubsystemRange> {
    let mut ranges = Vec::new();
    for attribute in attributes {
        if attribute.tag != b'A' {
            continue;
        }
        match parse_autominiroot(&attribute.text) {
            Some(range) => ranges.push(range),
            None => diagnostics.push(Diagnostic::warning(
                format!("unparsable autominiroot attribute {:?}", attribute.text),
                Some(origin.to_string()),
            )),
        }
    }
    ranges
}

/// Decodes the product cut point attributes (`C` blobs: a path, a
/// `0x01` separator and a sequence number).
pub(crate) fn cutpoints(
    attributes: &[DescriptorAttribute],
    origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Cutpoint> {
    let mut paths = Vec::new();
    for attribute in attributes {
        if attribute.tag != b'C' {
            continue;
        }
        match parse_cutpoint(&attribute.text) {
            Some(cutpoint) => paths.push(cutpoint),
            None => diagnostics.push(Diagnostic::warning(
                format!("unparsable cut point attribute {:?}", attribute.text),
                Some(origin.to_string()),
            )),
        }
    }
    paths
}

/// Decodes a plain (`bit` only), hardware-conditional (bit plus
/// expression blobs) or unresolved (unparsable blobs) flag.
fn conditional_flag(
    bit_set: bool,
    expressions: Vec<HardwareExpr>,
    unresolved: Vec<String>,
) -> ConditionalFlag {
    if !bit_set {
        ConditionalFlag::No
    } else if !unresolved.is_empty() {
        ConditionalFlag::WhenUnresolved {
            expressions,
            unresolved,
        }
    } else if expressions.is_empty() {
        ConditionalFlag::Always
    } else {
        ConditionalFlag::When(expressions)
    }
}

/// Parses the hardware expressions of every attribute with the given
/// tag, reporting unparsable ones as warnings and returning them
/// alongside the parsed expressions.
fn tagged_expressions(
    attributes: &[DescriptorAttribute],
    tag: u8,
    what: &str,
    origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<HardwareExpr>, Vec<String>) {
    let mut expressions = Vec::new();
    let mut unresolved = Vec::new();
    for attribute in attributes {
        if attribute.tag != tag {
            continue;
        }
        match HardwareExpr::parse(&attribute.text) {
            Ok(expression) => expressions.push(expression),
            Err(error) => {
                diagnostics.push(Diagnostic::warning(
                    format!("unparsable {what} expression {:?}: {error}", attribute.text),
                    Some(origin.to_string()),
                ));
                unresolved.push(attribute.text.clone());
            }
        }
    }
    (expressions, unresolved)
}

/// Decodes the prerequisite clauses, dropping a clause (with a warning)
/// when one of its ranges is not an ordinary range.
fn decode_prerequisites(
    clauses: &[Vec<DescriptorRange>],
    origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PrerequisiteClause> {
    let mut decoded = Vec::new();
    for clause in clauses {
        let mut all_of = Vec::new();
        for range in clause {
            match decode_ordinary(range, origin, diagnostics) {
                Some(versions) => all_of.push(range.to_logical(versions)),
                None => {
                    all_of.clear();
                    break;
                }
            }
        }
        if !all_of.is_empty() {
            decoded.push(PrerequisiteClause { all_of });
        }
    }
    decoded
}

/// Decodes an ordinary range, warning about the (unexpected) case where
/// the raw bounds do not form one.
fn decode_ordinary(
    range: &DescriptorRange,
    origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<VersionRange> {
    match range.ordinary_version_range() {
        Some(versions) => Some(versions),
        None => {
            diagnostics.push(Diagnostic::warning(
                format!(
                    "rule range {} has undecodable bounds {:#x}..{:#x}, skipped",
                    range.target(),
                    range.low_raw,
                    range.high_raw
                ),
                Some(origin.to_string()),
            ));
            None
        }
    }
}

/// Parses an `autominiroot` attribute payload: a dotted subsystem name
/// (patterns allowed) followed by decimal low and high version bounds.
fn parse_autominiroot(text: &str) -> Option<SubsystemRange> {
    let mut tokens = text.split_whitespace();
    let name = tokens.next()?;
    let low: u32 = tokens.next()?.parse().ok()?;
    let high: u32 = tokens.next()?.parse().ok()?;
    if tokens.next().is_some() {
        return None;
    }
    let mut parts = name.split('.');
    let product = parts.next()?;
    let image = parts.next()?;
    let subsystem = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some(SubsystemRange {
        product: product.to_string(),
        image: image.to_string(),
        subsystem: subsystem.to_string(),
        versions: VersionRange {
            min: Version(u64::from(low)),
            max: version_limit(high),
        },
    })
}

/// Parses a cut point attribute payload: a path, a `0x01` separator
/// and a decimal sequence number.
fn parse_cutpoint(text: &str) -> Option<Cutpoint> {
    let (path_text, sequence_text) = text.split_once('\x01')?;
    Some(Cutpoint {
        path: IrixPath::new(path_text).ok()?,
        sequence: sequence_text.parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::model::{Applicability, DescriptorSubsystem};
    use crate::mach::eval::HardwareProfile;

    fn attribute(tag: u8, text: &str) -> DescriptorAttribute {
        DescriptorAttribute {
            tag,
            text: text.to_string(),
        }
    }

    fn profile(cpuboard: &str) -> HardwareProfile {
        HardwareProfile::builder().set("CPUBOARD", cpuboard).build()
    }

    fn subsystem_record(
        flags_raw: u16,
        attributes: Vec<DescriptorAttribute>,
    ) -> DescriptorSubsystem {
        DescriptorSubsystem {
            flags_raw,
            name: "unix".to_string(),
            text: Default::default(),
            mapping: String::new(),
            legacy_ordinal: 0,
            replaces: Vec::new(),
            prerequisites: Vec::new(),
            incompatibilities: Vec::new(),
            attributes,
            updates: Vec::new(),
        }
    }

    #[test]
    fn mach_restrictions_are_fail_safe() {
        let mut diagnostics = Vec::new();

        // No attribute at all: no restriction.
        let none = mach_expressions(&[], "test", &mut diagnostics);
        assert!(none.is_empty());
        assert_eq!(none.evaluate(&profile("IP22")), Applicability::Match);

        // An unparsable expression must not fail open.
        let bad = mach_expressions(&[attribute(b'm', "=IP22")], "test", &mut diagnostics);
        assert_eq!(bad.unresolved, ["=IP22"]);
        assert_eq!(bad.evaluate(&profile("IP22")), Applicability::Unknown);

        // A parsed match wins even with unresolved siblings.
        let mixed = mach_expressions(
            &[attribute(b'm', "=IP22"), attribute(b'm', "CPUBOARD=IP22")],
            "test",
            &mut diagnostics,
        );
        assert_eq!(mixed.evaluate(&profile("IP22")), Applicability::Match);
        // ... but without a match, the unresolved expression rules.
        assert_eq!(mixed.evaluate(&profile("IP32")), Applicability::Unknown);
        assert!(!diagnostics.is_empty());
    }

    #[test]
    fn conditional_flags_decode_unresolved_conditions() {
        let mut diagnostics = Vec::new();

        // Bit set, no blobs: unconditional.
        let flags = decode_flags(0x0001 | 0x0800, &[], "test", &mut diagnostics);
        assert_eq!(flags.required, ConditionalFlag::Always);

        // Bit set, parsable blob: conditional.
        let flags = decode_flags(
            0x0002 | 0x0800,
            &[attribute(b'D', "CPUBOARD=IP22")],
            "test",
            &mut diagnostics,
        );
        let ConditionalFlag::When(expressions) = &flags.default else {
            panic!("expected conditional default");
        };
        assert_eq!(expressions.len(), 1);

        // Bit set, unparsable blob: unresolved, not "always".
        let flags = decode_flags(
            0x0001 | 0x0800,
            &[attribute(b'R', "=IP22")],
            "test",
            &mut diagnostics,
        );
        let ConditionalFlag::WhenUnresolved { unresolved, .. } = &flags.required else {
            panic!("expected unresolved required");
        };
        assert_eq!(unresolved, &["=IP22"]);
        assert_eq!(
            flags.required.is_active(&profile("IP22")),
            Applicability::Unknown
        );

        // Miniroot decodes from the cleared inplace bit.
        let flags = decode_flags(0x0050, &[], "test", &mut diagnostics);
        assert_eq!(flags.miniroot, ConditionalFlag::Always);
        assert!(!flags.inplace);
        let flags = decode_flags(0x0850, &[], "test", &mut diagnostics);
        assert_eq!(flags.miniroot, ConditionalFlag::No);
        assert!(flags.inplace);
    }

    #[test]
    fn rules_split_follows_from_replaces() {
        let mut record = subsystem_record(0x0400, Vec::new());
        record.replaces.push(DescriptorRange {
            product: "base".to_string(),
            image: "sw".to_string(),
            subsystem: "unix".to_string(),
            low_raw: (-1274567300i32) as u32,
            high_raw: 1274567300,
        });
        record.replaces.push(DescriptorRange {
            product: "patch*".to_string(),
            image: "sw".to_string(),
            subsystem: "unix".to_string(),
            low_raw: 0,
            high_raw: 99,
        });

        let mut diagnostics = Vec::new();
        let rules = decode_rules(&record, "test", &mut diagnostics);
        assert!(diagnostics.is_empty());
        assert_eq!(rules.follows.len(), 1);
        assert_eq!(rules.follows[0].target(), "base.sw.unix");
        assert_eq!(rules.follows[0].versions.min, Version(1274567300));
        assert_eq!(rules.replaces.len(), 1);
        assert_eq!(rules.replaces[0].target(), "patch*.sw.unix");
    }

    #[test]
    fn autominiroot_and_cutpoints_decode() {
        let mut diagnostics = Vec::new();

        let ranges = autominiroot(
            &[attribute(b'A', "eoe*.sw.unix 0 2147483647")],
            "test",
            &mut diagnostics,
        );
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].target(), "eoe*.sw.unix");
        assert!(matches!(
            ranges[0].versions.max,
            crate::descriptor::model::VersionLimit::Maximum
        ));

        let points = cutpoints(
            &[attribute(b'C', "/usr/share/catman\x017")],
            "test",
            &mut diagnostics,
        );
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].path.as_str(), "usr/share/catman");
        assert_eq!(points[0].sequence, 7);

        // Malformed payloads warn and are skipped.
        assert!(autominiroot(&[attribute(b'A', "bogus")], "test", &mut diagnostics).is_empty());
        assert!(cutpoints(&[attribute(b'C', "/usr")], "test", &mut diagnostics).is_empty());
        assert_eq!(diagnostics.len(), 2);
    }
}
