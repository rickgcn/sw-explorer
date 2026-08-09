//! Model of product descriptor data.
//!
//! IRIX `inst` uses its own integer version numbers (the ten-digit numbers
//! seen in installation conflict messages), not semantic versions.
//!
//! This module holds two layers:
//!
//! * the *physical* records ([`ProductDescriptor`], [`DescriptorImage`],
//!   [`DescriptorSubsystem`]), which mirror the binary `pd001` grammar
//!   field by field, exactly as `gendist` writes them;
//! * the *logical* tree types ([`Subsystem`], [`SubsystemFlags`],
//!   [`SubsystemRules`]) that the distribution model is built from.
//!
//! The logical layer carries *compiled* descriptor semantics: `gendist`
//! rewrites parts of the specification when it writes a descriptor (for
//! example `replaces ... maxint` becomes `version - 1`, and patch
//! subsystems lose their `required`/`default` flags), so a descriptor
//! is not a lossless decompilation of the source spec.
//!
//! Binary strings are length-prefixed Latin-1; they are decoded one
//! character per byte.
use crate::error::Result;
use crate::idb::EntryId;
use crate::mach::HardwareExpr;
use crate::mach::eval::HardwareProfile;
use crate::path::IrixPath;

/// The result of evaluating hardware-dependent semantics for a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applicability {
    /// Confirmed applicable.
    Match,
    /// Confirmed not applicable.
    NoMatch,
    /// Cannot be determined: at least one hardware expression could not
    /// be parsed, and none of the parsed ones matched.
    Unknown,
}

/// Hardware applicability of a product, image or subsystem: the
/// OR-ed `mach` expressions, plus any expression payloads that could
/// not be parsed.
///
/// The unresolved payloads are kept so that evaluation can be
/// fail-safe: an applicability that cannot be determined must never be
/// treated as "matches everything".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HardwareRestrictions {
    /// Parsed expressions (OR-ed).
    pub expressions: Vec<HardwareExpr>,
    /// Raw expression payloads that could not be parsed.
    pub unresolved: Vec<String>,
}

impl HardwareRestrictions {
    /// Whether nothing was declared at all (no restriction).
    pub fn is_empty(&self) -> bool {
        self.expressions.is_empty() && self.unresolved.is_empty()
    }

    /// Evaluates the restrictions for a target.
    pub fn evaluate(&self, profile: &HardwareProfile) -> Applicability {
        if self.expressions.iter().any(|e| e.evaluate(profile)) || self.is_empty() {
            Applicability::Match
        } else if !self.unresolved.is_empty() {
            Applicability::Unknown
        } else {
            Applicability::NoMatch
        }
    }
}

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

/// Converts a raw upper bound field into a [`VersionLimit`].
pub(crate) fn version_limit(high_raw: u32) -> VersionLimit {
    if high_raw == VERSION_UNBOUNDED_RAW {
        VersionLimit::Maximum
    } else {
        VersionLimit::Exact(Version(u64::from(high_raw)))
    }
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
/// The bounds are deliberately kept raw: `gendist` encodes patch
/// `follows` rules inside the `replaces` list by writing the low bound
/// as a *negative* signed 32-bit value, so an unsigned `low > high`
/// comparison is not corruption — it is the wire tag for `follows`.
/// Use [`DescriptorRange::is_follows`],
/// [`DescriptorRange::follows_version_range`] and
/// [`DescriptorRange::ordinary_version_range`] to decode; never sort or
/// normalize the raw fields.
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

    /// Whether this record is a `follows` entry: `gendist` writes the
    /// low bound of a patch `follows` rule as `-max(abs(low), 1)`, so
    /// the field interpreted as *signed* is negative.
    pub fn is_follows(&self) -> bool {
        (self.low_raw as i32) < 0
    }

    /// The range as an ordinary inclusive [`VersionRange`], or `None`
    /// for `follows` records (see [`DescriptorRange::is_follows`]) and
    /// for any other record whose raw bounds do not form an ordinary
    /// `low <= high` range.
    pub fn ordinary_version_range(&self) -> Option<VersionRange> {
        if self.is_follows() || self.low_raw > self.high_raw {
            return None;
        }
        Some(VersionRange {
            min: Version(u64::from(self.low_raw)),
            max: version_limit(self.high_raw),
        })
    }

    /// The decoded bounds of a `follows` record, or `None` when this is
    /// not one. The low bound is re-negated from its wire encoding.
    pub fn follows_version_range(&self) -> Option<VersionRange> {
        if !self.is_follows() {
            return None;
        }
        let low = (self.low_raw as i32).unsigned_abs();
        Some(VersionRange {
            min: Version(u64::from(low)),
            max: version_limit(self.high_raw),
        })
    }

    /// Builds the logical form of this range with the decoded bounds.
    pub(crate) fn to_logical(&self, versions: VersionRange) -> SubsystemRange {
        SubsystemRange {
            product: self.product.clone(),
            image: self.image.clone(),
            subsystem: self.subsystem.clone(),
            versions,
        }
    }
}

/// One `prereq` clause at the logical level.
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
    pub all_of: Vec<SubsystemRange>,
}

/// Dependency rules attached to a subsystem.
///
/// Every list is decoded from the descriptor record; an empty list means
/// the rule is confirmed absent. Note that `replaces` and `obsoletes`
/// compile into the same list and cannot be told apart afterwards.
///
/// The overlay/rule inheritance modifiers (`dontinheritreplaces`,
/// `dontinheritupdates`, `dontinheritincompat`, `dontinheritprereq`,
/// `inheritsoverlay`, `instbootstrap`, `bootstrapmandatory`) are *not*
/// decoded here; they remain available as raw [`DescriptorAttribute`]s
/// on the physical records.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubsystemRules {
    /// OR-ed prerequisite clauses; each clause is AND-ed internally.
    pub prerequisites: Vec<PrerequisiteClause>,
    /// Subsystems this one replaces (spec `replaces` and `obsoletes`).
    pub replaces: Vec<SubsystemRange>,
    /// Subsystems this one cannot coexist with (spec `incompat`).
    pub incompatibilities: Vec<SubsystemRange>,
    /// Subsystems this one updates (spec `updates`).
    pub updates: Vec<SubsystemRange>,
    /// The base subsystem a patch follows (spec `follows`).
    pub follows: Vec<SubsystemRange>,
}

/// A flag that can be plain (`default`) or hardware-conditional
/// (`default(CPUBOARD=IP22)`).
#[derive(Debug, Clone, Default, PartialEq)]
pub enum ConditionalFlag {
    /// Not set.
    #[default]
    No,
    /// Set unconditionally.
    Always,
    /// Set when any of these hardware expressions matches (OR-ed).
    When(Vec<HardwareExpr>),
    /// Set, but at least one condition expression could not be parsed,
    /// so whether the flag is active cannot always be determined.
    WhenUnresolved {
        /// Condition expressions that did parse (OR-ed).
        expressions: Vec<HardwareExpr>,
        /// Raw condition payloads that could not be parsed.
        unresolved: Vec<String>,
    },
}

impl ConditionalFlag {
    /// Whether the flag is set for a given installation target.
    pub fn is_active(&self, profile: &HardwareProfile) -> Applicability {
        match self {
            ConditionalFlag::No => Applicability::NoMatch,
            ConditionalFlag::Always => Applicability::Match,
            ConditionalFlag::When(expressions) => {
                if expressions.iter().any(|e| e.evaluate(profile)) {
                    Applicability::Match
                } else {
                    Applicability::NoMatch
                }
            }
            ConditionalFlag::WhenUnresolved {
                expressions,
                unresolved,
            } => {
                if expressions.iter().any(|e| e.evaluate(profile)) {
                    Applicability::Match
                } else if unresolved.is_empty() {
                    Applicability::NoMatch
                } else {
                    Applicability::Unknown
                }
            }
        }
    }
}

/// Installation flags of a subsystem, decoded from the raw flag word
/// and the conditional flag attribute blobs.
///
/// These are compiled semantics: `gendist` strips `required` and
/// `default` from patch subsystems when writing the descriptor, so a
/// patch's flags on disk are not necessarily what its spec declared.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubsystemFlags {
    /// Must always be installed.
    pub required: ConditionalFlag,
    /// Selected for installation by default.
    pub default: ConditionalFlag,
    /// Part of the miniroot.
    pub miniroot: ConditionalFlag,
    /// Installed in place on the target disk. This is the on-disk
    /// default; the bit is cleared for miniroot subsystems.
    pub inplace: bool,
    /// This subsystem is a patch.
    pub patch: bool,
    /// Installed on clients only (`clientonly`).
    pub client_only: bool,
    /// Overlay subsystem (`overlays`).
    pub overlay: bool,
    /// Member of an overlay product, skipped unless overlays are being
    /// processed.
    pub overlay_member: bool,
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
    /// Hardware applicability; `None` only for IDB-only subsystems,
    /// which have no descriptor record.
    pub mach: Option<HardwareRestrictions>,
    /// Installation flags; `None` only for IDB-only subsystems.
    pub flags: Option<SubsystemFlags>,
    /// `autominiroot` rule ranges; `None` only for IDB-only subsystems.
    pub autominiroot: Option<Vec<SubsystemRange>>,
    /// Dependency rules; `None` only for IDB-only subsystems, which have
    /// no descriptor record to decode rules from.
    pub rules: Option<SubsystemRules>,
    /// Entries belonging to this subsystem, in IDB order.
    pub entry_ids: Vec<EntryId>,
}

/// A decoded product descriptor header.
///
/// The generation id is the format generation the writing `gendist`
/// claimed (e.g. `pd001V630P00`); it is *not* the body layout version
/// and not the IRIX release of the CD the descriptor sits on — a single
/// distribution mixes several header variants. Readers dispatch on
/// [`ProductDescriptor::layout_level`] only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorHeader {
    /// The generation id, e.g. `pd001V630P00`.
    pub generation_id: String,
}

/// The two text fields every product, image and subsystem record
/// carries: the `id` one-liner and the long `desc`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DescriptorText {
    /// The one-line title (the spec `id` statement).
    pub id: String,
    /// The long description (the spec `desc` statement); empty on all
    /// observed media.
    pub description: String,
}

impl DescriptorText {
    /// The text a reader actually displays: `description`, falling back
    /// to `id` when it is empty.
    pub fn effective(&self) -> &str {
        if self.description.is_empty() {
            &self.id
        } else {
            &self.description
        }
    }
}

/// One tagged attribute blob from a product, image or subsystem record,
/// exactly as stored: a single tag byte plus tag-specific text.
///
/// Confirmed tags: `m` = hardware (`mach`) expression, `R`/`D`/`M` =
/// conditional `required`/`default`/`miniroot` expressions,
/// `A` = `autominiroot` range, `T` = disk-space accounting,
/// `C` = product cut point, `I` = inplace content marker, `P` = product
/// id. Unknown tags are preserved uninterpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorAttribute {
    /// The tag byte identifying the attribute kind.
    pub tag: u8,
    /// The tag payload.
    pub text: String,
}

impl DescriptorAttribute {
    /// The hardware expression this attribute carries, if it is a `mach`
    /// attribute (`m` tag, e.g. `mCPUBOARD=IP19 CPUBOARD=IP21`).
    pub fn mach(&self) -> Option<Result<HardwareExpr>> {
        (self.tag == b'm').then(|| HardwareExpr::parse(&self.text))
    }
}

/// A product cut point: a filesystem tree where installation may be
/// split, decoded from a `C` attribute blob.
#[derive(Debug, Clone, PartialEq)]
pub struct Cutpoint {
    /// Root of the splittable tree, e.g. `/usr/share/catman`.
    pub path: IrixPath,
    /// The sequence number recorded with the cut point.
    pub sequence: u32,
}

/// A subsystem record of a product descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct DescriptorSubsystem {
    /// Subsystem flag word, as stored. Known bits: `0x0001` required,
    /// `0x0002` default, `0x0400` patch, `0x0800` inplace (cleared for
    /// miniroot), `0x2000` clientonly, `0x4000` overlay member,
    /// `0x8000` overlays; older tools leave additional transient bits.
    pub flags_raw: u16,
    /// Short subsystem name, e.g. `unix`.
    pub name: String,
    /// Subsystem title and long description.
    pub text: DescriptorText,
    /// Mapping expression selecting which IDB records belong to this
    /// subsystem, e.g. `!noship && audio.data.sounds` or a plain tag
    /// such as `EOE`.
    pub mapping: String,
    /// Legacy per-subsystem ordinal. Current writers always store 0,
    /// but readers still consume the value, so it is a genuine wire
    /// field and is preserved as stored.
    pub legacy_ordinal: u32,
    /// The `replaces`/`obsoletes` list; on patch subsystems the
    /// `follows` records are mixed in front, encoded with a negated low
    /// bound (see [`DescriptorRange::is_follows`]).
    pub replaces: Vec<DescriptorRange>,
    /// `prereq` clauses; each inner list is one AND-ed clause.
    pub prerequisites: Vec<Vec<DescriptorRange>>,
    /// `incompat` ranges (layout level 6 and later).
    pub incompatibilities: Vec<DescriptorRange>,
    /// Tagged attribute blobs (layout level 8 and later).
    pub attributes: Vec<DescriptorAttribute>,
    /// `updates` ranges (layout level 9 and later).
    pub updates: Vec<DescriptorRange>,
}

/// An image record of a product descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct DescriptorImage {
    /// Image flag word; known bits: `0x0800` inplace, `0x4000`
    /// overlay-member.
    pub flags_raw: u16,
    /// Short image name, e.g. `sw`.
    pub name: String,
    /// Image title and long description.
    pub text: DescriptorText,
    /// Legacy field, always 2 on all observed media; its meaning is
    /// unknown.
    pub legacy_field: u16,
    /// Installation order (the spec `order` statement; default 9999).
    pub order: u16,
    /// Image version. Confirmed: replacement records targeting the
    /// subsystem itself use `version - 1` as their upper bound, matching
    /// the documented `oldvers` semantics.
    pub version: Version,
    /// Tagged attribute blobs (layout level 8 and later).
    pub attributes: Vec<DescriptorAttribute>,
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
    /// Body layout level (5 to 9): the minimum format level the product's
    /// features required when the descriptor was written.
    pub layout_level: u16,
    /// Product name as stored, e.g. `eoe`.
    pub name: String,
    /// Product title and long description.
    pub text: DescriptorText,
    /// Product flag word; known bits: `0x0800` inplace, `0x8000`
    /// overlay product.
    pub flags_raw: u16,
    /// Product stamp. Correlates with `P<stamp>_<n>` attribute blobs;
    /// the exact meaning is not confirmed, so this is deliberately not
    /// called a version.
    pub stamp: u32,
    /// Tagged attribute blobs (layout level 7 and later; level 7 carries
    /// `mach` blobs only).
    pub attributes: Vec<DescriptorAttribute>,
    /// Image records, in descriptor order.
    pub images: Vec<DescriptorImage>,
}
