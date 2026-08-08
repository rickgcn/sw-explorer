//! Logical model of product descriptor data.
//!
//! IRIX `inst` uses its own integer version numbers (the ten-digit numbers
//! seen in installation conflict messages), not semantic versions.
use crate::idb::EntryId;
use crate::mach::HardwareExpr;
use crate::mach::eval::HardwareProfile;
use crate::names::SubsystemName;

/// An SGI product version number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u64);

/// Upper bound of a version range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionLimit {
    /// Inclusive upper bound.
    Exact(Version),
    /// No upper bound.
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

/// A subsystem reference in a rule: exact, or a wildcard pattern such as
/// `eoe*.sw*`.
#[derive(Debug, Clone, PartialEq)]
pub enum SubsystemPattern {
    /// A single fully qualified subsystem.
    Exact(SubsystemName),
    /// A wildcard pattern matched against subsystem names.
    Pattern(String),
}

/// A subsystem reference plus the versions it applies to.
#[derive(Debug, Clone, PartialEq)]
pub struct SubsystemRange {
    /// Which subsystems the rule covers.
    pub name: SubsystemPattern,
    /// Which versions the rule covers.
    pub versions: VersionRange,
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
    pub all_of: Vec<SubsystemRange>,
}

/// Dependency rules attached to a subsystem.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubsystemRules {
    /// OR-ed prerequisite clauses; each clause is AND-ed internally.
    pub prerequisites: Vec<PrerequisiteClause>,
    /// Subsystems this one replaces.
    pub replaces: Vec<SubsystemRange>,
    /// Subsystems this one cannot coexist with.
    pub incompatibilities: Vec<SubsystemRange>,
    /// Subsystems this one updates.
    pub updates: Vec<SubsystemRange>,
    /// Subsystems this one follows (required on patches).
    pub follows: Vec<SubsystemRange>,
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

/// The smallest installable unit of a product.
///
/// Subsystems reference their entries by [`EntryId`]; the entries
/// themselves live in `Product::entries`.
#[derive(Debug, Clone)]
pub struct Subsystem {
    /// Fully qualified name, e.g. `eoe.sw.unix`.
    pub name: SubsystemName,
    /// Human-readable title from the descriptor.
    pub title: Option<String>,
    /// Hardware applicability expressions (OR-ed).
    pub mach: Vec<HardwareExpr>,
    /// Installation flags.
    pub flags: SubsystemFlags,
    /// `autominiroot` rule ranges.
    pub autominiroot: Vec<SubsystemRange>,
    /// Dependency rules.
    pub rules: SubsystemRules,
    /// Entries belonging to this subsystem, in IDB order.
    pub entry_ids: Vec<EntryId>,
}

/// A decoded product descriptor header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorHeader {
    /// The format magic, e.g. `pd001V630P00`.
    pub magic: String,
}

/// A product descriptor.
///
/// Only the header is decoded for now; the fields the body will provide
/// (titles, versions, flags, rules) already exist on `Image`,
/// [`Subsystem`] and the rule types above, so enriching the parser later
/// does not change the model.
#[derive(Debug, Clone)]
pub struct ProductDescriptor {
    /// Decoded header.
    pub header: DescriptorHeader,
}
