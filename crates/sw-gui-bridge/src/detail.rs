//! Construction of the detail DTOs declared by the CXX bridge from
//! `sw-core` domain objects.
//!
//! The conversions stay faithful to the domain model:
//!
//! * "unknown" (no descriptor record) is never conflated with
//!   "known but empty";
//! * unresolved MACH expressions are kept next to the parsed ones;
//! * conditional flags keep all four states, never a collapsed bool;
//! * prerequisite clauses keep their AND/OR two-level structure;
//! * an unbounded upper version bound (SGI `maxint`) is reported as
//!   such, never as a fake number;
//! * "known" is decided by descriptor-record presence, not by
//!   whether the decoded data happens to be empty.
use crate::bridge::ffi;
use sw_core::descriptor::model::{
    ConditionalFlag, HardwareRestrictions, Subsystem, SubsystemFlags, SubsystemRange,
    SubsystemRules, VersionLimit,
};
use sw_core::distribution::Product;
use sw_core::image::Image;

/// Converts hardware restrictions, keeping unknown (`None`) strictly
/// apart from known-but-unrestricted (`Some` with empty lists).
fn hardware_detail(mach: Option<&HardwareRestrictions>) -> ffi::HardwareDetail {
    match mach {
        Some(restrictions) => ffi::HardwareDetail {
            known: true,
            expressions: restrictions
                .expressions
                .iter()
                .map(|expression| expression.raw.clone())
                .collect(),
            unresolved: restrictions.unresolved.clone(),
        },
        None => ffi::HardwareDetail {
            known: false,
            expressions: Vec::new(),
            unresolved: Vec::new(),
        },
    }
}

/// Converts a conditional flag, preserving all four states.
fn conditional_flag_detail(flag: &ConditionalFlag) -> ffi::ConditionalFlagDetail {
    match flag {
        ConditionalFlag::No => ffi::ConditionalFlagDetail {
            state: ffi::ConditionalFlagState::No,
            expressions: Vec::new(),
            unresolved: Vec::new(),
        },
        ConditionalFlag::Always => ffi::ConditionalFlagDetail {
            state: ffi::ConditionalFlagState::Always,
            expressions: Vec::new(),
            unresolved: Vec::new(),
        },
        ConditionalFlag::When(expressions) => ffi::ConditionalFlagDetail {
            state: ffi::ConditionalFlagState::Conditional,
            expressions: expressions
                .iter()
                .map(|expression| expression.raw.clone())
                .collect(),
            unresolved: Vec::new(),
        },
        ConditionalFlag::WhenUnresolved {
            expressions,
            unresolved,
        } => ffi::ConditionalFlagDetail {
            state: ffi::ConditionalFlagState::Unresolved,
            expressions: expressions
                .iter()
                .map(|expression| expression.raw.clone())
                .collect(),
            unresolved: unresolved.clone(),
        },
    }
}

/// Converts subsystem flags, keeping IDB-only (`None`) strictly apart
/// from known flags.
fn flags_detail(flags: Option<&SubsystemFlags>) -> ffi::SubsystemFlagsDetail {
    match flags {
        Some(flags) => ffi::SubsystemFlagsDetail {
            known: true,
            required: conditional_flag_detail(&flags.required),
            default_flag: conditional_flag_detail(&flags.default),
            miniroot: conditional_flag_detail(&flags.miniroot),
            inplace: flags.inplace,
            patch: flags.patch,
            client_only: flags.client_only,
            overlay: flags.overlay,
            overlay_member: flags.overlay_member,
        },
        None => ffi::SubsystemFlagsDetail {
            known: false,
            required: conditional_flag_detail(&ConditionalFlag::No),
            default_flag: conditional_flag_detail(&ConditionalFlag::No),
            miniroot: conditional_flag_detail(&ConditionalFlag::No),
            inplace: false,
            patch: false,
            client_only: false,
            overlay: false,
            overlay_member: false,
        },
    }
}

/// Converts one rule range; an unbounded upper bound is reported via
/// `max_is_unbounded`, never as a fake number.
fn range_detail(range: &SubsystemRange) -> ffi::RangeDetail {
    let (max_is_unbounded, max_version) = match range.versions.max {
        VersionLimit::Exact(version) => (false, version.0),
        VersionLimit::Maximum => (true, 0),
    };
    ffi::RangeDetail {
        target: range.target(),
        min_version: range.versions.min.0,
        max_is_unbounded,
        max_version,
    }
}

/// Converts dependency rules, keeping IDB-only (`None`) strictly
/// apart from known-but-empty rule sets.
fn rules_detail(rules: Option<&SubsystemRules>) -> ffi::RulesDetail {
    match rules {
        Some(rules) => ffi::RulesDetail {
            known: true,
            prerequisites: rules
                .prerequisites
                .iter()
                .map(|clause| ffi::PrerequisiteClauseDetail {
                    all_of: clause.all_of.iter().map(range_detail).collect(),
                })
                .collect(),
            replaces: rules.replaces.iter().map(range_detail).collect(),
            incompatibilities: rules.incompatibilities.iter().map(range_detail).collect(),
            updates: rules.updates.iter().map(range_detail).collect(),
            follows: rules.follows.iter().map(range_detail).collect(),
        },
        None => ffi::RulesDetail {
            known: false,
            prerequisites: Vec::new(),
            replaces: Vec::new(),
            incompatibilities: Vec::new(),
            updates: Vec::new(),
            follows: Vec::new(),
        },
    }
}

/// Builds the detail DTO of the product with object id `id`.
pub(crate) fn product_detail(product: &Product, id: u64) -> ffi::ProductDetail {
    let descriptor = product.descriptor.as_ref();
    ffi::ProductDetail {
        id,
        name: product.name.as_str().to_string(),
        title: product.title.clone().unwrap_or_default(),
        descriptor_present: descriptor.is_some(),
        idb_present: product.idb_file.is_some(),
        image_count: product.images.len() as u64,
        subsystem_count: product
            .images
            .iter()
            .map(|image| image.subsystems.len() as u64)
            .sum(),
        entry_count: product.entries.len() as u64,
        descriptor_generation: descriptor
            .map(|d| d.header.generation_id.clone())
            .unwrap_or_default(),
        descriptor_layout_level: descriptor.map(|d| d.layout_level).unwrap_or(0),
        descriptor_stamp: descriptor.map(|d| d.stamp).unwrap_or(0),
        mach: hardware_detail(product.mach.as_ref()),
        cutpoints: product
            .cutpoints
            .iter()
            .map(|cutpoint| ffi::CutpointDetail {
                // IrixPath is root-relative; a cut point is an
                // install location, displayed as an absolute path.
                path: format!("/{}", cutpoint.path),
                sequence: cutpoint.sequence,
            })
            .collect(),
    }
}

/// Builds the detail DTO of the image with object id `id`.
pub(crate) fn image_detail(image: &Image, id: u64) -> ffi::ImageDetail {
    ffi::ImageDetail {
        id,
        name: image.name.file_name(),
        title: image.title.clone().unwrap_or_default(),
        product_name: image.name.product().as_str().to_string(),
        // Images synthesized from IDB entries carry no descriptor
        // record fields; this mirrors the hierarchy.
        descriptor_present: image.version.is_some(),
        idb_present: image
            .subsystems
            .iter()
            .any(|subsystem| subsystem.presence.idb),
        version_known: image.version.is_some(),
        version: image.version.map(|version| version.0).unwrap_or(0),
        order_known: image.order.is_some(),
        order: image.order.unwrap_or(0),
        subsystem_count: image.subsystems.len() as u64,
        entry_count: image
            .subsystems
            .iter()
            .map(|subsystem| subsystem.entry_ids.len() as u64)
            .sum(),
        mach: hardware_detail(image.mach.as_ref()),
    }
}

/// Builds the detail DTO of the subsystem with object id `id`;
/// `image` is the subsystem's containing image.
pub(crate) fn subsystem_detail(
    image: &Image,
    subsystem: &Subsystem,
    id: u64,
) -> ffi::SubsystemDetail {
    ffi::SubsystemDetail {
        id,
        identity: subsystem.name.to_string(),
        short_name: subsystem.name.subsystem().to_string(),
        title: subsystem.title.clone().unwrap_or_default(),
        product_name: subsystem.name.product().as_str().to_string(),
        image_name: subsystem.name.image_name().file_name(),
        image_version_known: image.version.is_some(),
        image_version: image.version.map(|version| version.0).unwrap_or(0),
        descriptor_present: subsystem.presence.descriptor,
        idb_present: subsystem.presence.idb,
        entry_count: subsystem.entry_ids.len() as u64,
        // Presence decides, not the mapping itself: a
        // descriptor-backed subsystem with an empty mapping is
        // "known to have no mapping", not "unknown".
        mapping_known: subsystem.presence.descriptor,
        mapping: subsystem.mapping.clone().unwrap_or_default(),
        mach: hardware_detail(subsystem.mach.as_ref()),
        flags: flags_detail(subsystem.flags.as_ref()),
        rules: rules_detail(subsystem.rules.as_ref()),
        autominiroot_known: subsystem.autominiroot.is_some(),
        autominiroot: subsystem
            .autominiroot
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(range_detail)
            .collect(),
    }
}
