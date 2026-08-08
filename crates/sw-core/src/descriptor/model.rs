//! Model of product descriptor data.
//!
//! IRIX `inst` uses its own integer version numbers (the ten-digit numbers
//! seen in installation conflict messages), not semantic versions.
//!
//! This module holds two layers:
//!
//! * the *physical* records ([`ProductDescriptor`], [`DescriptorImage`],
//!   [`DescriptorSubsystem`]), which mirror the binary `pd001` grammar
//!   field by field and keep raw values for everything whose semantics
//!   are not confirmed;
//! * the *logical* tree types ([`Subsystem`], [`SubsystemFlags`],
//!   [`SubsystemRules`]) that the distribution model is built from.
//!
//! Binary strings are length-prefixed Latin-1; they are decoded one
//! character per byte.
use crate::error::Result;
use crate::idb::EntryId;
use crate::mach::HardwareExpr;
use crate::mach::eval::HardwareProfile;

/// An SGI product version number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u64);

/// The raw wire value meaning "no upper version bound" (SGI `maxint`).
pub const VERSION_UNBOUNDED_RAW: u32 = 0x7fff_ffff;

/// Upper bound of a version range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionLimit {
    /// Inclusive upper bound.
    Exact(Version),
    /// No upper bound (SGI `maxint`, raw value
    /// [`VERSION_UNBOUNDED_RAW`]).
    Maximum,
}

/// An inclusive version range used by subsystem rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionRange {
    /// Lower bound.
    pub min: Version,
    /// Upper bound, or unbounded.
    pub max: VersionLimit,
}

/// A subsystem reference plus the versions it applies to, at the
/// *logical* level: the bounds are confirmed to be an ordinary
/// inclusive range.
///
/// The descriptor serializes the target as three independent fields, so
/// they are kept separate; each field may hold a wildcard pattern such
/// as product `patch*` or image `*`.
#[derive(Debug, Clone, PartialEq)]
pub struct SubsystemRange {
    /// Product name or pattern, e.g. `eoe1` or `patch*`.
    pub product: String,
    /// Image name or pattern, e.g. `sw`.
    pub image: String,
    /// Subsystem name or pattern, e.g. `unix`.
    pub subsystem: String,
    /// Which versions the rule covers.
    pub versions: VersionRange,
}

impl SubsystemRange {
    /// The dotted target as written in SGI documentation, e.g.
    /// `patch*.sw.unix`.
    pub fn target(&self) -> String {
        format!("{}.{}.{}", self.product, self.image, self.subsystem)
    }
}

/// One raw range record exactly as stored in a descriptor:
/// `LP16 product, LP16 image, LP16 subsystem, BE32 low, BE32 high`.
///
/// The bounds are deliberately *not* normalized into a
/// [`VersionRange`]: real media contain records with `low > high`
/// whose 32-bit fields carry an encoding that is not yet understood,
/// so claiming `min <= max` would be a lie. Use
/// [`DescriptorRange::ordinary_version_range`] for the confirmed case.
#[derive(Debug, Clone, PartialEq)]
pub struct DescriptorRange {
    /// Product name or pattern, e.g. `eoe1` or `patch*`.
    pub product: String,
    /// Image name or pattern, e.g. `sw`.
    pub image: String,
    /// Subsystem name or pattern, e.g. `unix`.
    pub subsystem: String,
    /// The raw lower bound field.
    pub low_raw: u32,
    /// The raw upper bound field; [`VERSION_UNBOUNDED_RAW`] is SGI
    /// `maxint`.
    pub high_raw: u32,
}

impl DescriptorRange {
    /// The dotted target as written in SGI documentation, e.g.
    /// `patch*.sw.unix`.
    pub fn target(&self) -> String {
        format!("{}.{}.{}", self.product, self.image, self.subsystem)
    }

    /// Whether the raw upper bound is SGI `maxint` (unbounded).
    pub fn high_is_maxint(&self) -> bool {
        self.high_raw == VERSION_UNBOUNDED_RAW
    }

    /// The range as an ordinary inclusive [`VersionRange`], or `None`
    /// when `low_raw > high_raw`, which real records exhibit and whose
    /// encoding is not yet understood.
    pub fn ordinary_version_range(&self) -> Option<VersionRange> {
        if self.low_raw > self.high_raw {
            return None;
        }
        Some(VersionRange {
            min: Version(u64::from(self.low_raw)),
            max: if self.high_is_maxint() {
                VersionLimit::Maximum
            } else {
                VersionLimit::Exact(Version(u64::from(self.high_raw)))
            },
        })
    }
}

/// One `prereq` clause.
///
/// SGI prerequisite semantics are two-level: several ranges inside one
/// clause are AND-ed, while separate clauses are OR-ed:
///
/// ```text
/// prereq(A B)
/// prereq(C)
/// ```
///
/// means `(A AND B) OR C`, which is exactly
/// `Vec<PrerequisiteClause>` where each clause holds `all_of`.
#[derive(Debug, Clone, PartialEq)]
pub struct PrerequisiteClause {
    /// Ranges that must all be satisfied together.
    pub all_of: Vec<DescriptorRange>,
}

/// Dependency rules attached to a subsystem.
///
/// Only prerequisites are decoded so far; every other rule list is
/// `None` — *not decoded yet* — rather than an empty list, which would
/// wrongly claim the rule is confirmed absent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubsystemRules {
    /// OR-ed prerequisite clauses; each clause is AND-ed internally.
    /// Decoded from descriptor rule slot 3; empty means confirmed none.
    pub prerequisites: Vec<PrerequisiteClause>,
    /// Subsystems this one replaces; `None` = not decoded yet.
    pub replaces: Option<Vec<SubsystemRange>>,
    /// Subsystems this one cannot coexist with; `None` = not decoded
    /// yet.
    pub incompatibilities: Option<Vec<SubsystemRange>>,
    /// Subsystems this one updates; `None` = not decoded yet.
    pub updates: Option<Vec<SubsystemRange>>,
    /// Subsystems this one follows (required on patches); `None` = not
    /// decoded yet.
    pub follows: Option<Vec<SubsystemRange>>,
}

/// A flag that can be plain (`default`) or hardware-conditional
/// (`default(CPUBOARD=IP22)`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConditionalFlag {
    /// Set unconditionally.
    pub unconditional: bool,
    /// Hardware expressions under which the flag is set.
    pub conditions: Vec<HardwareExpr>,
}

impl ConditionalFlag {
    /// Whether the flag is set for a given installation target.
    pub fn is_active(&self, profile: &HardwareProfile) -> bool {
        self.unconditional || self.conditions.iter().any(|e| e.evaluate(profile))
    }
}

/// Installation flags of a subsystem.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubsystemFlags {
    /// Must always be installed.
    pub required: ConditionalFlag,
    /// Selected for installation by default.
    pub default: ConditionalFlag,
    /// Part of the miniroot.
    pub miniroot: ConditionalFlag,
    /// This subsystem is a patch.
    pub patch: bool,
}

/// Which authorities a subsystem was found in.
///
/// Real media can declare a subsystem in the descriptor without shipping
/// any of its entries in the IDB (e.g. unshipped hardware support), and
/// in principle an IDB could reference a subsystem the descriptor does
/// not declare. Both are kept, not "fixed".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubsystemPresence {
    /// Declared by the product descriptor.
    pub descriptor: bool,
    /// Referenced by at least one IDB entry.
    pub idb: bool,
}

/// The smallest installable unit of a product.
///
/// Subsystems reference their entries by [`EntryId`]; the entries
/// themselves live in `Product::entries`.
#[derive(Debug, Clone)]
pub struct Subsystem {
    /// Fully qualified name, e.g. `eoe.sw.unix`.
    pub name: crate::names::SubsystemName,
    /// Human-readable title from the descriptor.
    pub title: Option<String>,
    /// Mapping expression selecting which IDB records belong to this
    /// subsystem, from the descriptor; `None` for IDB-only subsystems.
    pub mapping: Option<String>,
    /// Which authorities declare this subsystem.
    pub presence: SubsystemPresence,
    /// Hardware applicability expressions (OR-ed); `None` = not decoded
    /// from the descriptor yet, which is *not* the same as "no
    /// restriction".
    pub mach: Option<Vec<HardwareExpr>>,
    /// Installation flags; `None` = the raw flag word is not decoded
    /// yet.
    pub flags: Option<SubsystemFlags>,
    /// `autominiroot` rule ranges; `None` = not decoded yet.
    pub autominiroot: Option<Vec<SubsystemRange>>,
    /// Dependency rules; `None` for IDB-only subsystems, which have no
    /// descriptor record to decode rules from.
    pub rules: Option<SubsystemRules>,
    /// Entries belonging to this subsystem, in IDB order.
    pub entry_ids: Vec<EntryId>,
}

/// A decoded product descriptor header.
///
/// The magic is kept verbatim: the `VxxxPxx` numbers record the format
/// the descriptor was generated with, not the IRIX release of the CD it
/// sits on (a single distribution mixes several header variants).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorHeader {
    /// The format magic, e.g. `pd001V630P00`.
    pub magic: String,
}

/// One length-prefixed metadata blob from a product or image record.
///
/// The raw bytes are always authoritative; [`DescriptorMetadata::mach`]
/// offers a parsed view of the one blob kind whose meaning is confirmed
/// (`m` followed by a hardware expression). Blobs with other leading
/// bytes (`I`, `P`, ...) are preserved uninterpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorMetadata {
    /// The blob bytes, exactly as stored.
    pub raw: Vec<u8>,
}

impl DescriptorMetadata {
    /// The hardware expression this blob carries, if it is a `mach`
    /// blob (`m` prefix followed by expression text, e.g.
    /// `mCPUBOARD=IP19 CPUBOARD=IP21`).
    pub fn mach(&self) -> Option<Result<HardwareExpr>> {
        let text = self.raw.strip_prefix(b"m")?;
        let text: String = text.iter().map(|&b| b as char).collect();
        Some(HardwareExpr::parse(&text))
    }
}

/// A subsystem record of a product descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct DescriptorSubsystem {
    /// Subsystem flag word; the bit meanings are not yet confirmed.
    pub flags_raw: u16,
    /// Short subsystem name, e.g. `unix`.
    pub name: String,
    /// Subsystem description (title text).
    pub description: String,
    /// Mapping expression selecting which IDB records belong to this
    /// subsystem, e.g. `!noship && audio.data.sounds` or a plain tag
    /// such as `EOE`.
    pub mapping: String,
    /// Rule slot 3: prerequisites, OR-ed clauses of AND-ed ranges.
    pub prerequisites: Vec<PrerequisiteClause>,
    /// Non-empty rule slots whose semantics are not yet confirmed
    /// (slots 2, 5, 7 and 8), slot numbers preserved.
    pub unassigned_slots: Vec<RuleSlot>,
}

/// A non-empty subsystem rule slot with unconfirmed semantics.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleSlot {
    /// A slot of flat range records.
    Ranges {
        /// Slot index within the rules region.
        slot: u8,
        /// Range records, in file order.
        values: Vec<DescriptorRange>,
    },
    /// A slot of plain strings.
    Strings {
        /// Slot index within the rules region.
        slot: u8,
        /// Strings, in file order.
        values: Vec<String>,
    },
}

impl RuleSlot {
    /// The slot index within the rules region.
    pub fn slot(&self) -> u8 {
        match self {
            RuleSlot::Ranges { slot, .. } | RuleSlot::Strings { slot, .. } => *slot,
        }
    }
}

/// An image record of a product descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct DescriptorImage {
    /// Image flag word; the bit meanings are not yet confirmed.
    pub flags_raw: u16,
    /// Short image name, e.g. `sw`.
    pub name: String,
    /// Image description (title text).
    pub description: String,
    /// Unknown field between the description and the order candidate.
    pub unknown_a: u16,
    /// Strong installation-order candidate: observed values (600, 900,
    /// 9999, ...) match the documented installation-order semantics, but
    /// this is not confirmed.
    pub install_order_raw: u16,
    /// Image version. Confirmed by corpus: replacement records targeting
    /// the subsystem itself use `version - 1` as their upper bound,
    /// matching the documented `oldvers` semantics.
    pub version: Version,
    /// Extra unknown fields in the record tail (layout level 5 only).
    pub tail_unknowns: Vec<u16>,
    /// Metadata blobs (layout levels 5, 8 and 9; level 6 has none).
    pub metadata: Vec<DescriptorMetadata>,
    /// Subsystem records, in descriptor order.
    pub subsystems: Vec<DescriptorSubsystem>,
}

/// A fully parsed product descriptor.
///
/// The descriptor is the authoritative source of the
/// product → image → subsystem hierarchy; the IDB only contributes the
/// file entries attached to that tree.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductDescriptor {
    /// Decoded header.
    pub header: DescriptorHeader,
    /// Body layout level (5, 6, 8 or 9); the rules region of each
    /// subsystem holds exactly this many slots.
    pub layout_level: u16,
    /// Product name as stored, e.g. `eoe`.
    pub name: String,
    /// Product description (title text).
    pub description: String,
    /// Product flag word; the bit meanings are not yet confirmed.
    pub flags_raw: u16,
    /// Product stamp. Correlates with `P<stamp>_<n>` metadata blobs;
    /// the exact meaning is not confirmed, so this is deliberately not
    /// called a version.
    pub stamp: u32,
    /// Metadata blobs (layout levels 8 and 9 only).
    pub metadata: Vec<DescriptorMetadata>,
    /// Image records, in descriptor order.
    pub images: Vec<DescriptorImage>,
}
