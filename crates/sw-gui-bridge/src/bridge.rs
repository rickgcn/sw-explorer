//! The CXX bridge itself: an opaque [`Backend`] handle plus the plain
//! data structs returned across the boundary.
//!
//! All `sw-core` errors are propagated to C++ as CXX exceptions; the
//! bridge contains no business logic of its own.
use sw_core::descriptor::model::{
    ConditionalFlag, HardwareRestrictions, SubsystemFlags, SubsystemRange, SubsystemRules,
    VersionLimit,
};
use sw_core::distribution::Distribution;

#[cxx::bridge(namespace = "sw")]
mod ffi {
    /// Which level of the product → image → subsystem tree a node sits
    /// on.
    enum ObjectKind {
        Product,
        Image,
        Subsystem,
    }

    /// Counts describing a freshly opened distribution.
    struct DistributionSummary {
        /// Number of products found in the distribution.
        product_count: u64,
        /// Number of non-fatal problems reported while opening it.
        diagnostic_count: u64,
    }

    /// One node of the distribution hierarchy.
    ///
    /// The hierarchy crosses the bridge as a flat list in natural
    /// order: products in distribution order, each followed by its
    /// images in descriptor order, each followed by its subsystems.
    /// Parents therefore always precede their children.
    struct HierarchyNode {
        /// Stable, non-zero object id. Zero is reserved for the Qt-side
        /// invisible root and never identifies a real object.
        id: u64,
        /// Id of the parent node, or zero for products.
        parent_id: u64,
        /// Tree level of this node.
        kind: ObjectKind,

        /// Display name: the product name (`eoe`), the image file name
        /// (`eoe.sw`) or the short subsystem name (`unix`).
        name: String,
        /// Human-readable title, empty when the object has none.
        title: String,

        /// Number of entries: everything below the product, everything
        /// below the image, or the subsystem's own entries.
        entry_count: u64,

        /// Whether the product descriptor declares this object.
        descriptor_present: bool,
        /// Whether the product IDB references this object.
        idb_present: bool,
    }

    /// The state of a hardware-conditional installation flag.
    ///
    /// Mirrors the four states of
    /// `sw_core::descriptor::model::ConditionalFlag`; a conditional or
    /// unresolved flag is never collapsed into a bool.
    enum ConditionalFlagState {
        /// Not set.
        No,
        /// Set unconditionally.
        Always,
        /// Set when any of the listed hardware expressions matches.
        Conditional,
        /// Set, but at least one condition could not be parsed.
        Unresolved,
    }

    /// Hardware applicability of a product, image or subsystem.
    struct HardwareDetail {
        /// `false` when the object has no descriptor record and its
        /// MACH restrictions are therefore unknown. `true` with both
        /// lists empty means the descriptor confirms there is no
        /// restriction; the two cases are never conflated.
        known: bool,
        /// Parsed expression texts (OR-ed), exactly as written.
        expressions: Vec<String>,
        /// Raw expression payloads that could not be parsed.
        unresolved: Vec<String>,
    }

    /// One installation flag that may be hardware-conditional.
    struct ConditionalFlagDetail {
        /// Which state the flag is in.
        state: ConditionalFlagState,
        /// Condition expression texts (OR-ed), when conditional.
        expressions: Vec<String>,
        /// Raw condition payloads that could not be parsed.
        unresolved: Vec<String>,
    }

    /// The installation flags of a subsystem.
    struct SubsystemFlagsDetail {
        /// `false` for IDB-only subsystems: without a descriptor
        /// record the flags are unknown, not "all clear".
        known: bool,

        /// Must always be installed.
        required: ConditionalFlagDetail,
        /// Selected for installation by default.
        default_flag: ConditionalFlagDetail,
        /// Part of the miniroot.
        miniroot: ConditionalFlagDetail,

        /// Installed in place on the target disk.
        inplace: bool,
        /// This subsystem is a patch.
        patch: bool,
        /// Installed on clients only.
        client_only: bool,
        /// Overlay subsystem.
        overlay: bool,
        /// Member of an overlay product.
        overlay_member: bool,
    }

    /// One versioned rule range, e.g. `patch*.sw.unix 0..1274627332`.
    struct RangeDetail {
        /// The dotted target, e.g. `patch*.sw.unix`.
        target: String,
        /// Inclusive lower version bound.
        min_version: u64,

        /// Whether the range is unbounded above (SGI `maxint`); when
        /// set, `max_version` carries no meaning.
        max_is_unbounded: bool,
        /// Inclusive upper version bound, unless unbounded.
        max_version: u64,
    }

    /// One `prereq` clause: ranges that must all be satisfied
    /// together. Separate clauses are alternatives (OR); the bridge
    /// deliberately keeps this two-level structure.
    struct PrerequisiteClauseDetail {
        /// Ranges that must all be satisfied together (AND).
        all_of: Vec<RangeDetail>,
    }

    /// The dependency rules of a subsystem.
    struct RulesDetail {
        /// `false` for IDB-only subsystems: without a descriptor
        /// record the rules are unknown. `true` with every list empty
        /// means the descriptor confirms there are no rules.
        known: bool,

        /// OR-ed prerequisite clauses, each AND-ed internally.
        prerequisites: Vec<PrerequisiteClauseDetail>,
        /// Subsystems this one replaces.
        replaces: Vec<RangeDetail>,
        /// Subsystems this one cannot coexist with.
        incompatibilities: Vec<RangeDetail>,
        /// Subsystems this one updates.
        updates: Vec<RangeDetail>,
        /// The base subsystem a patch follows.
        follows: Vec<RangeDetail>,
    }

    /// A product cut point: a filesystem tree where installation may
    /// be split.
    struct CutpointDetail {
        /// Root of the splittable tree, e.g. `/usr/share/catman`.
        path: String,
        /// The sequence number recorded with the cut point.
        sequence: u32,
    }

    /// Everything the inspector shows for one product.
    struct ProductDetail {
        /// The object id this detail was requested with.
        id: u64,

        /// Product name, e.g. `eoe`.
        name: String,
        /// Human-readable title, empty when the product has none.
        title: String,

        /// Whether the product has a parsed descriptor.
        descriptor_present: bool,
        /// Whether the product has an IDB file.
        idb_present: bool,

        /// Number of images in the product.
        image_count: u64,
        /// Number of subsystems across all images.
        subsystem_count: u64,
        /// Number of entries across the whole product.
        entry_count: u64,

        /// Descriptor generation id, e.g. `pd001V630P00`; empty when
        /// there is no descriptor.
        descriptor_generation: String,
        /// Descriptor body layout level; meaningless without a
        /// descriptor.
        descriptor_layout_level: u16,
        /// Product stamp; meaningless without a descriptor.
        descriptor_stamp: u32,

        /// Hardware applicability from the descriptor's `mach` blobs.
        mach: HardwareDetail,
        /// Cut points recorded in the descriptor.
        cutpoints: Vec<CutpointDetail>,
    }

    /// Everything the inspector shows for one image.
    struct ImageDetail {
        /// The object id this detail was requested with.
        id: u64,

        /// Image file name, e.g. `eoe.sw`.
        name: String,
        /// Human-readable title, empty when the image has none.
        title: String,
        /// Name of the containing product, e.g. `eoe`.
        product_name: String,

        /// Whether the image comes from the product descriptor.
        descriptor_present: bool,
        /// Whether any of its subsystems is referenced by the IDB.
        idb_present: bool,

        /// Whether the image carries a product version; `false` for
        /// IDB-derived images.
        version_known: bool,
        /// The product version, when known.
        version: u64,

        /// Whether the image carries an installation order hint.
        order_known: bool,
        /// The installation order hint, when known.
        order: i32,

        /// Number of subsystems in the image.
        subsystem_count: u64,
        /// Number of entries across the whole image.
        entry_count: u64,

        /// Hardware applicability from the descriptor's `mach` blobs.
        mach: HardwareDetail,
    }

    /// Everything the inspector shows for one subsystem.
    struct SubsystemDetail {
        /// The object id this detail was requested with.
        id: u64,

        /// Fully qualified name, e.g. `eoe.sw.unix`.
        identity: String,
        /// Short subsystem name, e.g. `unix`.
        short_name: String,
        /// Human-readable title, empty when the subsystem has none.
        title: String,

        /// Name of the containing product, e.g. `eoe`.
        product_name: String,
        /// Name of the containing image, e.g. `eoe.sw`.
        image_name: String,

        /// Whether the containing image carries a product version.
        image_version_known: bool,
        /// The containing image's product version, when known.
        image_version: u64,

        /// Whether the product descriptor declares this subsystem.
        descriptor_present: bool,
        /// Whether the product IDB references this subsystem.
        idb_present: bool,

        /// Number of entries belonging to this subsystem.
        entry_count: u64,

        /// Whether the mapping is known: `true` exactly when the
        /// subsystem has a descriptor record. A descriptor-backed
        /// subsystem with an empty mapping is "known to have no
        /// mapping", which is not the same as unknown.
        mapping_known: bool,
        /// The mapping expression, empty when there is none.
        mapping: String,

        /// Hardware applicability from the descriptor's `mach` blobs.
        mach: HardwareDetail,
        /// Installation flags.
        flags: SubsystemFlagsDetail,
        /// Dependency rules.
        rules: RulesDetail,

        /// Whether `autominiroot` rules are known (descriptor-backed).
        autominiroot_known: bool,
        /// The `autominiroot` rule ranges, when known.
        autominiroot: Vec<RangeDetail>,
    }

    extern "Rust" {
        /// Opaque handle to the Rust backend state.
        type Backend;

        /// Creates an empty backend with no distribution loaded.
        fn new_backend() -> Box<Backend>;

        /// Opens the distribution directory at `path`.
        ///
        /// The previously loaded distribution is replaced only after
        /// the new one has been opened successfully; a failed open
        /// leaves the old state untouched.
        fn open_distribution(self: &mut Backend, path: &str) -> Result<DistributionSummary>;

        /// Flattens the loaded distribution's product → image →
        /// subsystem hierarchy into natural order.
        ///
        /// Fails when no distribution has been loaded.
        fn hierarchy(self: &Backend) -> Result<Vec<HierarchyNode>>;

        /// Returns the detail snapshot of the product with object id
        /// `id`.
        ///
        /// Fails when no distribution is loaded, when `id` is zero or
        /// unknown, or when `id` identifies an image or subsystem.
        fn product_detail(self: &Backend, id: u64) -> Result<ProductDetail>;

        /// Returns the detail snapshot of the image with object id
        /// `id`.
        ///
        /// Fails when no distribution is loaded, when `id` is zero or
        /// unknown, or when `id` identifies a product or subsystem.
        fn image_detail(self: &Backend, id: u64) -> Result<ImageDetail>;

        /// Returns the detail snapshot of the subsystem with object id
        /// `id`.
        ///
        /// Fails when no distribution is loaded, when `id` is zero or
        /// unknown, or when `id` identifies a product or image.
        fn subsystem_detail(self: &Backend, id: u64) -> Result<SubsystemDetail>;
    }
}

/// Identifies one domain object of the loaded distribution by its
/// position: product index, image index and subsystem index into
/// `Distribution::products()` and below.
#[derive(Clone, Copy)]
enum ObjectRef {
    Product(usize),
    Image(usize, usize),
    Subsystem(usize, usize, usize),
}

/// One entry of the backend's object table: the public object id is
/// the entry's position in the table plus one.
struct ObjectEntry {
    /// Object id of the parent entry, or zero for products.
    parent_id: u64,
    /// Which domain object this entry refers to.
    reference: ObjectRef,
}

/// Error for bridge operations that need a loaded distribution.
#[derive(Debug)]
pub struct NoDistributionLoaded;

impl std::fmt::Display for NoDistributionLoaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no distribution loaded")
    }
}

impl std::error::Error for NoDistributionLoaded {}

/// Error for object detail lookups: no distribution loaded, or an
/// object id that is zero, unknown or of the wrong kind.
#[derive(Debug)]
pub struct ObjectDetailError(String);

impl std::fmt::Display for ObjectDetailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ObjectDetailError {}

/// Rust-side backend state behind the opaque CXX handle.
pub struct Backend {
    distribution: Option<Distribution>,
    /// Object table of the loaded distribution: stable, non-zero ids
    /// for every product, image and subsystem. Rebuilt on every
    /// successful open; a failed open leaves it untouched.
    objects: Vec<ObjectEntry>,
}

fn new_backend() -> Box<Backend> {
    Box::new(Backend {
        distribution: None,
        objects: Vec::new(),
    })
}

/// Builds the object table of a freshly opened distribution, assigning
/// sequential non-zero ids in natural (depth-first) order.
fn build_object_table(distribution: &Distribution) -> Vec<ObjectEntry> {
    let mut objects = Vec::new();
    for (product_index, product) in distribution.products().iter().enumerate() {
        let product_id = objects.len() as u64 + 1;
        objects.push(ObjectEntry {
            parent_id: 0,
            reference: ObjectRef::Product(product_index),
        });
        for (image_index, image) in product.images.iter().enumerate() {
            let image_id = objects.len() as u64 + 1;
            objects.push(ObjectEntry {
                parent_id: product_id,
                reference: ObjectRef::Image(product_index, image_index),
            });
            for subsystem_index in 0..image.subsystems.len() {
                objects.push(ObjectEntry {
                    parent_id: image_id,
                    reference: ObjectRef::Subsystem(product_index, image_index, subsystem_index),
                });
            }
        }
    }
    objects
}

/// The tree-level name of an object reference, for error messages.
fn kind_name(reference: &ObjectRef) -> &'static str {
    match reference {
        ObjectRef::Product(_) => "a product",
        ObjectRef::Image(..) => "an image",
        ObjectRef::Subsystem(..) => "a subsystem",
    }
}

/// The error for a detail request aimed at the wrong tree level.
fn wrong_kind(id: u64, expected: &str, reference: &ObjectRef) -> ObjectDetailError {
    ObjectDetailError(format!(
        "object {id} is {}, not {expected}",
        kind_name(reference)
    ))
}

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

impl Backend {
    /// Resolves an object id to its table entry, validating that a
    /// distribution is loaded and that the id is in range. No
    /// indexing past this point can panic: the returned reference is
    /// derived from the entry, not the id.
    fn resolve_object(&self, id: u64) -> Result<(&ObjectRef, &Distribution), ObjectDetailError> {
        let distribution = self
            .distribution
            .as_ref()
            .ok_or_else(|| ObjectDetailError("no distribution loaded".to_string()))?;
        if id == 0 {
            return Err(ObjectDetailError(
                "object id 0 does not identify a distribution object".to_string(),
            ));
        }
        let entry = self
            .objects
            .get((id - 1) as usize)
            .ok_or_else(|| ObjectDetailError(format!("object {id} does not exist")))?;
        Ok((&entry.reference, distribution))
    }

    fn open_distribution(
        &mut self,
        path: &str,
    ) -> sw_core::error::Result<ffi::DistributionSummary> {
        // Open first, replace second: a failed open must not destroy
        // the distribution that is already loaded.
        let distribution = Distribution::open(path)?;
        let summary = ffi::DistributionSummary {
            product_count: distribution.products().len() as u64,
            diagnostic_count: distribution.diagnostics().len() as u64,
        };
        let objects = build_object_table(&distribution);
        self.distribution = Some(distribution);
        self.objects = objects;
        Ok(summary)
    }

    fn hierarchy(&self) -> std::result::Result<Vec<ffi::HierarchyNode>, NoDistributionLoaded> {
        let distribution = self.distribution.as_ref().ok_or(NoDistributionLoaded)?;
        let mut nodes = Vec::with_capacity(self.objects.len());
        for (index, entry) in self.objects.iter().enumerate() {
            let id = index as u64 + 1;
            let parent_id = entry.parent_id;
            let node = match entry.reference {
                ObjectRef::Product(product_index) => {
                    let product = &distribution.products()[product_index];
                    ffi::HierarchyNode {
                        id,
                        parent_id,
                        kind: ffi::ObjectKind::Product,
                        name: product.name.as_str().to_string(),
                        title: product.title.clone().unwrap_or_default(),
                        entry_count: product.entries.len() as u64,
                        descriptor_present: product.descriptor.is_some(),
                        idb_present: product.idb_file.is_some(),
                    }
                }
                ObjectRef::Image(product_index, image_index) => {
                    let image = &distribution.products()[product_index].images[image_index];
                    ffi::HierarchyNode {
                        id,
                        parent_id,
                        kind: ffi::ObjectKind::Image,
                        name: image.name.file_name(),
                        title: image.title.clone().unwrap_or_default(),
                        entry_count: image
                            .subsystems
                            .iter()
                            .map(|subsystem| subsystem.entry_ids.len() as u64)
                            .sum(),
                        // Images synthesized from IDB entries carry no
                        // descriptor record fields.
                        descriptor_present: image.version.is_some(),
                        idb_present: image
                            .subsystems
                            .iter()
                            .any(|subsystem| subsystem.presence.idb),
                    }
                }
                ObjectRef::Subsystem(product_index, image_index, subsystem_index) => {
                    let subsystem = &distribution.products()[product_index].images[image_index]
                        .subsystems[subsystem_index];
                    ffi::HierarchyNode {
                        id,
                        parent_id,
                        kind: ffi::ObjectKind::Subsystem,
                        name: subsystem.name.subsystem().to_string(),
                        title: subsystem.title.clone().unwrap_or_default(),
                        entry_count: subsystem.entry_ids.len() as u64,
                        descriptor_present: subsystem.presence.descriptor,
                        idb_present: subsystem.presence.idb,
                    }
                }
            };
            nodes.push(node);
        }
        Ok(nodes)
    }

    fn product_detail(&self, id: u64) -> Result<ffi::ProductDetail, ObjectDetailError> {
        let (reference, distribution) = self.resolve_object(id)?;
        let ObjectRef::Product(product_index) = *reference else {
            return Err(wrong_kind(id, "a product", reference));
        };
        let product = &distribution.products()[product_index];
        let descriptor = product.descriptor.as_ref();
        Ok(ffi::ProductDetail {
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
        })
    }

    fn image_detail(&self, id: u64) -> Result<ffi::ImageDetail, ObjectDetailError> {
        let (reference, distribution) = self.resolve_object(id)?;
        let ObjectRef::Image(product_index, image_index) = *reference else {
            return Err(wrong_kind(id, "an image", reference));
        };
        let image = &distribution.products()[product_index].images[image_index];
        Ok(ffi::ImageDetail {
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
        })
    }

    fn subsystem_detail(&self, id: u64) -> Result<ffi::SubsystemDetail, ObjectDetailError> {
        let (reference, distribution) = self.resolve_object(id)?;
        let ObjectRef::Subsystem(product_index, image_index, subsystem_index) = *reference else {
            return Err(wrong_kind(id, "a subsystem", reference));
        };
        let image = &distribution.products()[product_index].images[image_index];
        let subsystem = &image.subsystems[subsystem_index];
        Ok(ffi::SubsystemDetail {
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn temp_root(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "sw-gui-bridge-test-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// Writes a minimal synthetic distribution: one valid product
    /// (`test`, IDB only) plus one product whose file name is not a
    /// valid product name, which surfaces as a distribution-level
    /// diagnostic.
    fn write_synthetic_dist(root: &Path) {
        std::fs::write(
            root.join("test.idb"),
            "f 0644 root sys hello.txt src/hello.txt test.sw.unix sum(1) size(5) cmpsize(0)\n",
        )
        .unwrap();
        std::fs::write(root.join("not.a.product.idb"), "garbage\n").unwrap();
    }

    /// Writes an IDB for `product` with one plain file record per
    /// `(subsystem, path)` pair.
    fn write_idb(root: &Path, product: &str, entries: &[(&str, &str)]) {
        let mut idb = String::new();
        for (subsystem, path) in entries {
            idb.push_str(&format!(
                "f 0644 root sys {path} src/{path} {subsystem} sum(1) size(5) cmpsize(0)\n"
            ));
        }
        std::fs::write(root.join(format!("{product}.idb")), idb).unwrap();
    }

    /// Builds a minimal level-9 descriptor for a product with a single
    /// `sw` image containing the given subsystems.
    fn write_descriptor(root: &Path, product: &str, subsystems: &[&str]) {
        fn lp16(bytes: &mut Vec<u8>, s: &str) {
            bytes.extend_from_slice(&(s.len() as u16).to_be_bytes());
            bytes.extend_from_slice(s.as_bytes());
        }

        let mut bytes = b"pd001V999P00\0".to_vec();
        for word in [0x07c4u16, 0x0001, 0x07c3, 9] {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
        lp16(&mut bytes, product);
        lp16(&mut bytes, "Test Product");
        bytes.extend_from_slice(&0x0850u16.to_be_bytes()); // product flags
        bytes.extend_from_slice(&1u32.to_be_bytes()); // stamp
        bytes.extend_from_slice(&0u32.to_be_bytes()); // reserved
        bytes.extend_from_slice(&0u16.to_be_bytes()); // metadata count
        bytes.extend_from_slice(&1u16.to_be_bytes()); // image count

        bytes.extend_from_slice(&0x0858u16.to_be_bytes()); // image flags
        lp16(&mut bytes, "sw");
        lp16(&mut bytes, "System Software");
        bytes.extend_from_slice(&0u16.to_be_bytes()); // legacy field
        bytes.extend_from_slice(&9999u16.to_be_bytes()); // order
        bytes.extend_from_slice(&1u32.to_be_bytes()); // version
        bytes.extend_from_slice(&0u32.to_be_bytes()); // reserved
        bytes.extend_from_slice(&0u16.to_be_bytes()); // metadata count
        bytes.extend_from_slice(&(subsystems.len() as u16).to_be_bytes());

        for subsystem in subsystems {
            bytes.extend_from_slice(&0x0852u16.to_be_bytes()); // flags
            lp16(&mut bytes, subsystem);
            lp16(&mut bytes, "Subsystem Title");
            lp16(&mut bytes, "ALL");
            for _ in 0..9 {
                bytes.extend_from_slice(&0u16.to_be_bytes()); // rule slots
            }
        }
        std::fs::write(root.join(product), bytes).unwrap();
    }

    /// One raw rule range of a synthetic descriptor: a dotted target
    /// plus the raw wire bounds, so even the negated-low `follows`
    /// encoding can be written.
    struct RangeSpec {
        target: &'static str,
        low: u32,
        high: u32,
    }

    /// One subsystem record of a synthetic descriptor.
    struct SubsystemSpec {
        name: &'static str,
        title: &'static str,
        mapping: &'static str,
        flags: u16,
        replaces: Vec<RangeSpec>,
        prerequisites: Vec<Vec<RangeSpec>>,
        incompat: Vec<RangeSpec>,
        updates: Vec<RangeSpec>,
        /// Tag byte plus payload bytes (payloads may contain control
        /// characters, e.g. cut point separators).
        attributes: Vec<(u8, Vec<u8>)>,
    }

    impl SubsystemSpec {
        fn plain(
            name: &'static str,
            title: &'static str,
            mapping: &'static str,
            flags: u16,
        ) -> Self {
            SubsystemSpec {
                name,
                title,
                mapping,
                flags,
                replaces: Vec::new(),
                prerequisites: Vec::new(),
                incompat: Vec::new(),
                updates: Vec::new(),
                attributes: Vec::new(),
            }
        }
    }

    /// The raw wire value meaning "no upper version bound".
    const MAXINT: u32 = 0x7fff_ffff;

    fn push_lp16(bytes: &mut Vec<u8>, text: &str) {
        bytes.extend_from_slice(&(text.len() as u16).to_be_bytes());
        bytes.extend_from_slice(text.as_bytes());
    }

    fn push_range(bytes: &mut Vec<u8>, range: &RangeSpec) {
        let parts: Vec<&str> = range.target.split('.').collect();
        assert_eq!(parts.len(), 3, "range target must be dotted");
        for part in parts {
            push_lp16(bytes, part);
        }
        bytes.extend_from_slice(&range.low.to_be_bytes());
        bytes.extend_from_slice(&range.high.to_be_bytes());
    }

    fn push_range_list(bytes: &mut Vec<u8>, ranges: &[RangeSpec]) {
        bytes.extend_from_slice(&(ranges.len() as u16).to_be_bytes());
        for range in ranges {
            push_range(bytes, range);
        }
    }

    fn push_attributes(bytes: &mut Vec<u8>, attributes: &[(u8, Vec<u8>)]) {
        bytes.extend_from_slice(&(attributes.len() as u32).to_be_bytes());
        for (tag, payload) in attributes {
            bytes.extend_from_slice(&(payload.len() as u16 + 1).to_be_bytes());
            bytes.push(*tag);
            bytes.extend_from_slice(payload);
        }
    }

    /// Writes a level-9 descriptor with one image, exposing every
    /// field the detail APIs decode: product attributes (mach, cut
    /// points), image version/order and full subsystem records with
    /// rules and attributes.
    #[allow(clippy::too_many_arguments)]
    fn write_rich_descriptor(
        root: &Path,
        product: &str,
        title: &str,
        stamp: u32,
        product_attributes: Vec<(u8, Vec<u8>)>,
        image: &str,
        image_title: &str,
        image_order: u16,
        image_version: u32,
        subsystems: &[SubsystemSpec],
    ) {
        let mut bytes = b"pd001V999P00\0".to_vec();
        for word in [0x07c4u16, 0x0001, 0x07c3, 9] {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
        push_lp16(&mut bytes, product);
        push_lp16(&mut bytes, title);
        bytes.extend_from_slice(&0x0850u16.to_be_bytes()); // product flags
        bytes.extend_from_slice(&stamp.to_be_bytes());
        push_lp16(&mut bytes, ""); // desc
        push_attributes(&mut bytes, &product_attributes);
        bytes.extend_from_slice(&1u16.to_be_bytes()); // image count

        bytes.extend_from_slice(&0x0858u16.to_be_bytes()); // image flags
        push_lp16(&mut bytes, image);
        push_lp16(&mut bytes, image_title);
        bytes.extend_from_slice(&0u16.to_be_bytes()); // legacy field
        bytes.extend_from_slice(&image_order.to_be_bytes());
        bytes.extend_from_slice(&image_version.to_be_bytes());
        push_lp16(&mut bytes, ""); // desc
        push_attributes(&mut bytes, &[]);
        bytes.extend_from_slice(&(subsystems.len() as u16).to_be_bytes());

        for subsystem in subsystems {
            bytes.extend_from_slice(&subsystem.flags.to_be_bytes());
            push_lp16(&mut bytes, subsystem.name);
            push_lp16(&mut bytes, subsystem.title);
            push_lp16(&mut bytes, subsystem.mapping);
            bytes.extend_from_slice(&0u32.to_be_bytes()); // legacy ordinal
            push_range_list(&mut bytes, &subsystem.replaces);
            bytes.extend_from_slice(&(subsystem.prerequisites.len() as u16).to_be_bytes());
            for clause in &subsystem.prerequisites {
                push_range_list(&mut bytes, clause);
            }
            push_lp16(&mut bytes, ""); // desc
            push_range_list(&mut bytes, &subsystem.incompat);
            push_attributes(&mut bytes, &subsystem.attributes);
            push_range_list(&mut bytes, &subsystem.updates);
        }
        std::fs::write(root.join(product), bytes).unwrap();
    }

    /// Writes the standard "rich" test distribution: one descriptor+
    /// IDB product `rich` whose `unix` subsystem exercises every
    /// decoded feature, whose `ghost` subsystem is descriptor-only
    /// with known-but-empty data, and whose IDB adds the IDB-only
    /// subsystem `extra`.
    ///
    /// Object ids: 1 = product `rich`, 2 = image `rich.sw`,
    /// 3 = `unix`, 4 = `ghost`, 5 = `extra`.
    fn write_rich_dist(root: &Path) {
        let unix = SubsystemSpec {
            name: "unix",
            title: "UNIX Execution Environment",
            mapping: "ALL",
            // required + default bits; the inplace bit is clear, which
            // makes miniroot plain "Always".
            flags: 0x0053,
            replaces: vec![
                RangeSpec {
                    target: "patch*.sw.unix",
                    low: 0,
                    high: 1_274_627_332,
                },
                // Negated low bound: the wire encoding of `follows`.
                RangeSpec {
                    target: "eoe.sw.base",
                    low: (-5i32) as u32,
                    high: 100,
                },
            ],
            prerequisites: vec![
                vec![
                    RangeSpec {
                        target: "eoe.sw.gfx",
                        low: 1,
                        high: 10,
                    },
                    RangeSpec {
                        target: "eoe.sw.unix",
                        low: 2,
                        high: 20,
                    },
                ],
                vec![RangeSpec {
                    target: "foo.sw.bar",
                    low: 0,
                    high: MAXINT,
                }],
            ],
            incompat: vec![RangeSpec {
                target: "evil.sw.bad",
                low: 0,
                high: MAXINT,
            }],
            updates: vec![RangeSpec {
                target: "old.sw.thing",
                low: 3,
                high: 9,
            }],
            attributes: vec![
                (b'R', b"=BROKEN".to_vec()),
                (b'D', b"CPUBOARD=IP22".to_vec()),
                (b'm', b"GFXBOARD=NEWPRESS".to_vec()),
                (b'm', b"GFXBOARD=EXPRESS".to_vec()),
                (b'm', b"=GARBAGE".to_vec()),
                (b'A', b"eoe.sw.unix 0 1274627332".to_vec()),
            ],
        };
        // Inplace bit only: every conditional flag decodes to `No`.
        let ghost = SubsystemSpec::plain("ghost", "Ghost Subsystem", "", 0x0850);

        write_rich_descriptor(
            root,
            "rich",
            "Rich Product",
            424_242,
            vec![
                (b'm', b"CPUBOARD=IP22".to_vec()),
                (b'C', b"/usr/share/catman\x013".to_vec()),
            ],
            "sw",
            "System Software",
            3,
            42,
            &[unix, ghost],
        );
        write_idb(
            root,
            "rich",
            &[
                ("rich.sw.unix", "a"),
                ("rich.sw.unix", "b"),
                ("rich.sw.extra", "c"),
            ],
        );
    }

    /// The stable shape of one hierarchy node, for comparisons.
    fn node_shape(node: &ffi::HierarchyNode) -> (u64, u64, &'static str, String, u64, bool, bool) {
        let kind = match node.kind {
            ffi::ObjectKind::Product => "product",
            ffi::ObjectKind::Image => "image",
            ffi::ObjectKind::Subsystem => "subsystem",
            _ => "unknown",
        };
        (
            node.id,
            node.parent_id,
            kind,
            node.name.to_string(),
            node.entry_count,
            node.descriptor_present,
            node.idb_present,
        )
    }

    #[test]
    fn new_backend_has_no_distribution() {
        let backend = new_backend();
        assert!(backend.distribution.is_none());
    }

    #[test]
    fn hierarchy_requires_loaded_distribution() {
        let backend = new_backend();
        assert!(backend.hierarchy().is_err());
    }

    #[test]
    fn open_missing_directory_fails() {
        let mut backend = new_backend();
        let missing = std::env::temp_dir().join("sw-gui-bridge-test-definitely-missing");
        assert!(!missing.exists());
        let result = backend.open_distribution(missing.to_str().unwrap());
        assert!(result.is_err());
        assert!(backend.distribution.is_none());
    }

    #[test]
    fn open_synthetic_distribution_reports_counts() {
        let root = temp_root("synthetic");
        write_synthetic_dist(&root);

        let mut backend = new_backend();
        let summary = backend.open_distribution(root.to_str().unwrap()).unwrap();
        assert_eq!(summary.product_count, 1);
        assert_eq!(summary.diagnostic_count, 1);
        assert!(backend.distribution.is_some());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_open_keeps_previous_distribution() {
        let root = temp_root("keep");
        write_synthetic_dist(&root);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();

        let missing = root.join("missing");
        assert!(
            backend
                .open_distribution(missing.to_str().unwrap())
                .is_err()
        );
        assert!(backend.distribution.is_some());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hierarchy_flattens_idb_only_tree() {
        let root = temp_root("idb-tree");
        write_idb(
            &root,
            "test",
            &[
                ("test.sw.unix", "a"),
                ("test.sw.unix", "b"),
                ("test.man.man", "c"),
            ],
        );

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();
        let nodes = backend.hierarchy().unwrap();

        let shapes: Vec<_> = nodes.iter().map(node_shape).collect();
        assert_eq!(
            shapes,
            vec![
                (1, 0, "product", "test".to_string(), 3, false, true),
                (2, 1, "image", "test.sw".to_string(), 2, false, true),
                (3, 2, "subsystem", "unix".to_string(), 2, false, true),
                (4, 1, "image", "test.man".to_string(), 1, false, true),
                (5, 4, "subsystem", "man".to_string(), 1, false, true),
            ]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hierarchy_ids_are_nonzero_unique_and_stable() {
        let root = temp_root("ids");
        write_idb(&root, "test", &[("test.sw.unix", "a")]);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();

        let first = backend.hierarchy().unwrap();
        let second = backend.hierarchy().unwrap();

        assert!(first.iter().all(|node| node.id != 0));
        let mut ids: Vec<u64> = first.iter().map(|node| node.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), first.len());

        let first_ids: Vec<u64> = first.iter().map(|node| node.id).collect();
        let second_ids: Vec<u64> = second.iter().map(|node| node.id).collect();
        assert_eq!(first_ids, second_ids);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hierarchy_includes_descriptor_only_objects() {
        let root = temp_root("descriptor-only");
        // No IDB at all: the whole tree comes from the descriptor, and
        // `ghost` is a subsystem no IDB entry could ever reference.
        write_descriptor(&root, "desconly", &["unix", "ghost"]);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();
        let nodes = backend.hierarchy().unwrap();

        let shapes: Vec<_> = nodes.iter().map(node_shape).collect();
        assert_eq!(
            shapes,
            vec![
                (1, 0, "product", "desconly".to_string(), 0, true, false),
                (2, 1, "image", "desconly.sw".to_string(), 0, true, false),
                (3, 2, "subsystem", "unix".to_string(), 0, true, false),
                (4, 2, "subsystem", "ghost".to_string(), 0, true, false),
            ]
        );
        assert_eq!(nodes[0].title.to_string(), "Test Product");
        assert_eq!(nodes[2].title.to_string(), "Subsystem Title");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hierarchy_includes_idb_only_objects() {
        let root = temp_root("idb-only");
        // The descriptor declares only `unix`; the IDB additionally
        // references `extra`, which becomes an IDB-only subsystem.
        write_descriptor(&root, "mixed", &["unix"]);
        write_idb(
            &root,
            "mixed",
            &[("mixed.sw.unix", "a"), ("mixed.sw.extra", "b")],
        );

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();
        let nodes = backend.hierarchy().unwrap();

        let shapes: Vec<_> = nodes.iter().map(node_shape).collect();
        assert_eq!(
            shapes,
            vec![
                (1, 0, "product", "mixed".to_string(), 2, true, true),
                (2, 1, "image", "mixed.sw".to_string(), 2, true, true),
                (3, 2, "subsystem", "unix".to_string(), 1, true, true),
                (4, 2, "subsystem", "extra".to_string(), 1, false, true),
            ]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_open_keeps_previous_hierarchy() {
        let root = temp_root("keep-hierarchy");
        write_idb(&root, "test", &[("test.sw.unix", "a")]);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();
        let before: Vec<_> = backend
            .hierarchy()
            .unwrap()
            .iter()
            .map(node_shape)
            .collect();

        let missing = root.join("missing");
        assert!(
            backend
                .open_distribution(missing.to_str().unwrap())
                .is_err()
        );

        let after: Vec<_> = backend
            .hierarchy()
            .unwrap()
            .iter()
            .map(node_shape)
            .collect();
        assert_eq!(before, after);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn successful_reopen_replaces_hierarchy() {
        let root_a = temp_root("reopen-a");
        write_idb(&root_a, "alpha", &[("alpha.sw.unix", "a")]);
        let root_b = temp_root("reopen-b");
        write_idb(&root_b, "beta", &[("beta.man.man", "b")]);

        let mut backend = new_backend();
        backend.open_distribution(root_a.to_str().unwrap()).unwrap();
        backend.open_distribution(root_b.to_str().unwrap()).unwrap();

        let nodes = backend.hierarchy().unwrap();
        let names: Vec<String> = nodes.iter().map(|node| node.name.to_string()).collect();
        assert_eq!(names, vec!["beta", "beta.man", "man"]);

        let _ = std::fs::remove_dir_all(&root_a);
        let _ = std::fs::remove_dir_all(&root_b);
    }

    #[test]
    fn detail_requires_loaded_distribution() {
        let backend = new_backend();
        assert_eq!(
            backend.product_detail(1).err().unwrap().to_string(),
            "no distribution loaded"
        );
        assert_eq!(
            backend.image_detail(1).err().unwrap().to_string(),
            "no distribution loaded"
        );
        assert_eq!(
            backend.subsystem_detail(1).err().unwrap().to_string(),
            "no distribution loaded"
        );
    }

    #[test]
    fn detail_rejects_zero_and_out_of_range_ids() {
        let root = temp_root("detail-bad-ids");
        write_idb(&root, "test", &[("test.sw.unix", "a")]);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();
        // Three objects: product, image, subsystem.
        assert_eq!(backend.hierarchy().unwrap().len(), 3);

        assert_eq!(
            backend.product_detail(0).err().unwrap().to_string(),
            "object id 0 does not identify a distribution object"
        );
        assert_eq!(
            backend.image_detail(0).err().unwrap().to_string(),
            "object id 0 does not identify a distribution object"
        );
        assert_eq!(
            backend.subsystem_detail(4).err().unwrap().to_string(),
            "object 4 does not exist"
        );
        assert_eq!(
            backend.product_detail(1000).err().unwrap().to_string(),
            "object 1000 does not exist"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn detail_rejects_wrong_object_kind() {
        let root = temp_root("detail-wrong-kind");
        write_idb(&root, "test", &[("test.sw.unix", "a")]);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();

        assert_eq!(
            backend.product_detail(2).err().unwrap().to_string(),
            "object 2 is an image, not a product"
        );
        assert_eq!(
            backend.product_detail(3).err().unwrap().to_string(),
            "object 3 is a subsystem, not a product"
        );
        assert_eq!(
            backend.image_detail(1).err().unwrap().to_string(),
            "object 1 is a product, not an image"
        );
        assert_eq!(
            backend.subsystem_detail(1).err().unwrap().to_string(),
            "object 1 is a product, not a subsystem"
        );
        assert_eq!(
            backend.subsystem_detail(2).err().unwrap().to_string(),
            "object 2 is an image, not a subsystem"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn product_detail_of_idb_only_product_keeps_unknown() {
        let root = temp_root("detail-idb-product");
        write_idb(
            &root,
            "test",
            &[("test.sw.unix", "a"), ("test.man.man", "b")],
        );

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();
        let detail = backend.product_detail(1).unwrap();

        assert_eq!(detail.id, 1);
        assert_eq!(detail.name, "test");
        assert_eq!(detail.title, "");
        assert!(!detail.descriptor_present);
        assert!(detail.idb_present);
        assert_eq!(detail.image_count, 2);
        assert_eq!(detail.subsystem_count, 2);
        assert_eq!(detail.entry_count, 2);
        // No descriptor: metadata carries no meaning and MACH is
        // unknown, not "unrestricted".
        assert_eq!(detail.descriptor_generation, "");
        assert!(!detail.mach.known);
        assert!(detail.mach.expressions.is_empty());
        assert!(detail.mach.unresolved.is_empty());
        assert!(detail.cutpoints.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn product_detail_decodes_descriptor_metadata() {
        let root = temp_root("detail-rich-product");
        write_rich_dist(&root);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();
        let detail = backend.product_detail(1).unwrap();

        assert_eq!(detail.name, "rich");
        assert_eq!(detail.title, "Rich Product");
        assert!(detail.descriptor_present);
        assert!(detail.idb_present);
        assert_eq!(detail.image_count, 1);
        assert_eq!(detail.subsystem_count, 3);
        assert_eq!(detail.entry_count, 3);
        assert_eq!(detail.descriptor_generation, "pd001V999P00");
        assert_eq!(detail.descriptor_layout_level, 9);
        assert_eq!(detail.descriptor_stamp, 424_242);

        assert!(detail.mach.known);
        assert_eq!(detail.mach.expressions, vec!["CPUBOARD=IP22".to_string()]);
        assert!(detail.mach.unresolved.is_empty());

        assert_eq!(detail.cutpoints.len(), 1);
        assert_eq!(detail.cutpoints[0].path, "/usr/share/catman");
        assert_eq!(detail.cutpoints[0].sequence, 3);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn image_detail_decodes_descriptor_fields() {
        let root = temp_root("detail-rich-image");
        write_rich_dist(&root);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();
        let detail = backend.image_detail(2).unwrap();

        assert_eq!(detail.id, 2);
        assert_eq!(detail.name, "rich.sw");
        assert_eq!(detail.title, "System Software");
        assert_eq!(detail.product_name, "rich");
        assert!(detail.descriptor_present);
        assert!(detail.idb_present);
        assert!(detail.version_known);
        assert_eq!(detail.version, 42);
        assert!(detail.order_known);
        assert_eq!(detail.order, 3);
        assert_eq!(detail.subsystem_count, 3);
        assert_eq!(detail.entry_count, 3);
        // The image record carries no mach attributes: known to be
        // unrestricted, which differs from unknown.
        assert!(detail.mach.known);
        assert!(detail.mach.expressions.is_empty());
        assert!(detail.mach.unresolved.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn image_detail_of_idb_only_image_keeps_unknown() {
        let root = temp_root("detail-idb-image");
        write_idb(&root, "test", &[("test.sw.unix", "a")]);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();
        let detail = backend.image_detail(2).unwrap();

        assert_eq!(detail.name, "test.sw");
        assert!(!detail.descriptor_present);
        assert!(detail.idb_present);
        assert!(!detail.version_known);
        assert!(!detail.order_known);
        assert!(!detail.mach.known);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn subsystem_detail_decodes_full_descriptor_record() {
        let root = temp_root("detail-rich-subsystem");
        write_rich_dist(&root);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();
        let detail = backend.subsystem_detail(3).unwrap();

        assert_eq!(detail.id, 3);
        assert_eq!(detail.identity, "rich.sw.unix");
        assert_eq!(detail.short_name, "unix");
        assert_eq!(detail.title, "UNIX Execution Environment");
        assert_eq!(detail.product_name, "rich");
        assert_eq!(detail.image_name, "rich.sw");
        assert!(detail.image_version_known);
        assert_eq!(detail.image_version, 42);
        assert!(detail.descriptor_present);
        assert!(detail.idb_present);
        assert_eq!(detail.entry_count, 2);

        assert!(detail.mapping_known);
        assert_eq!(detail.mapping, "ALL");

        // MACH: two parsed expressions plus the unresolved payload,
        // which must never be dropped.
        assert!(detail.mach.known);
        assert_eq!(
            detail.mach.expressions,
            vec![
                "GFXBOARD=NEWPRESS".to_string(),
                "GFXBOARD=EXPRESS".to_string()
            ]
        );
        assert_eq!(detail.mach.unresolved, vec!["=GARBAGE".to_string()]);

        // Flags: all four conditional states in one subsystem.
        assert!(detail.flags.known);
        assert!(matches!(
            detail.flags.required.state,
            ffi::ConditionalFlagState::Unresolved
        ));
        assert_eq!(
            detail.flags.required.unresolved,
            vec!["=BROKEN".to_string()]
        );
        assert!(matches!(
            detail.flags.default_flag.state,
            ffi::ConditionalFlagState::Conditional
        ));
        assert_eq!(
            detail.flags.default_flag.expressions,
            vec!["CPUBOARD=IP22".to_string()]
        );
        assert!(matches!(
            detail.flags.miniroot.state,
            ffi::ConditionalFlagState::Always
        ));
        assert!(!detail.flags.inplace);
        assert!(!detail.flags.patch);
        assert!(!detail.flags.client_only);
        assert!(!detail.flags.overlay);
        assert!(!detail.flags.overlay_member);

        // Rules: prerequisite clauses keep their AND/OR structure.
        assert!(detail.rules.known);
        assert_eq!(detail.rules.prerequisites.len(), 2);
        let first: Vec<&str> = detail.rules.prerequisites[0]
            .all_of
            .iter()
            .map(|range| range.target.as_str())
            .collect();
        assert_eq!(first, vec!["eoe.sw.gfx", "eoe.sw.unix"]);
        assert_eq!(detail.rules.prerequisites[0].all_of[0].min_version, 1);
        assert_eq!(detail.rules.prerequisites[0].all_of[0].max_version, 10);
        assert!(!detail.rules.prerequisites[0].all_of[0].max_is_unbounded);
        assert_eq!(detail.rules.prerequisites[1].all_of.len(), 1);
        assert_eq!(detail.rules.prerequisites[1].all_of[0].target, "foo.sw.bar");
        assert!(detail.rules.prerequisites[1].all_of[0].max_is_unbounded);

        assert_eq!(detail.rules.replaces.len(), 1);
        assert_eq!(detail.rules.replaces[0].target, "patch*.sw.unix");
        assert_eq!(detail.rules.replaces[0].min_version, 0);
        assert_eq!(detail.rules.replaces[0].max_version, 1_274_627_332);
        assert!(!detail.rules.replaces[0].max_is_unbounded);

        // The negated-low record decodes into follows, not replaces.
        assert_eq!(detail.rules.follows.len(), 1);
        assert_eq!(detail.rules.follows[0].target, "eoe.sw.base");
        assert_eq!(detail.rules.follows[0].min_version, 5);
        assert_eq!(detail.rules.follows[0].max_version, 100);

        assert_eq!(detail.rules.incompatibilities.len(), 1);
        assert!(detail.rules.incompatibilities[0].max_is_unbounded);

        assert_eq!(detail.rules.updates.len(), 1);
        assert_eq!(detail.rules.updates[0].target, "old.sw.thing");
        assert_eq!(detail.rules.updates[0].min_version, 3);
        assert_eq!(detail.rules.updates[0].max_version, 9);

        assert!(detail.autominiroot_known);
        assert_eq!(detail.autominiroot.len(), 1);
        assert_eq!(detail.autominiroot[0].target, "eoe.sw.unix");
        assert_eq!(detail.autominiroot[0].max_version, 1_274_627_332);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn subsystem_detail_distinguishes_known_empty_from_unknown() {
        let root = temp_root("detail-known-vs-unknown");
        write_rich_dist(&root);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();

        // `ghost`: descriptor-backed, so every descriptor-derived
        // field is *known*; an empty mapping or empty rule set is a
        // fact, not a gap.
        let ghost = backend.subsystem_detail(4).unwrap();
        assert!(ghost.descriptor_present);
        assert!(!ghost.idb_present);
        assert_eq!(ghost.entry_count, 0);
        assert!(ghost.mapping_known);
        assert_eq!(ghost.mapping, "");
        assert!(ghost.mach.known);
        assert!(ghost.mach.expressions.is_empty());
        assert!(ghost.flags.known);
        assert!(matches!(
            ghost.flags.default_flag.state,
            ffi::ConditionalFlagState::No
        ));
        assert!(ghost.flags.inplace);
        assert!(ghost.rules.known);
        assert!(ghost.rules.prerequisites.is_empty());
        assert!(ghost.rules.replaces.is_empty());
        assert!(ghost.autominiroot_known);
        assert!(ghost.autominiroot.is_empty());

        // `extra`: IDB-only, so every descriptor-derived field is
        // *unknown* — never reported as empty.
        let extra = backend.subsystem_detail(5).unwrap();
        assert!(!extra.descriptor_present);
        assert!(extra.idb_present);
        assert_eq!(extra.entry_count, 1);
        assert!(!extra.mapping_known);
        assert!(!extra.mach.known);
        assert!(!extra.flags.known);
        assert!(!extra.rules.known);
        assert!(!extra.autominiroot_known);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_open_keeps_previous_detail_queryable() {
        let root = temp_root("detail-keep");
        write_rich_dist(&root);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();
        let before = backend.subsystem_detail(3).unwrap();

        let missing = root.join("missing");
        assert!(
            backend
                .open_distribution(missing.to_str().unwrap())
                .is_err()
        );

        let after = backend.subsystem_detail(3).unwrap();
        assert_eq!(after.identity, before.identity);
        assert_eq!(after.rules.prerequisites.len(), 2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn successful_reopen_queries_new_distribution() {
        let root_a = temp_root("detail-reopen-a");
        write_idb(&root_a, "alpha", &[("alpha.sw.unix", "a")]);
        let root_b = temp_root("detail-reopen-b");
        write_idb(&root_b, "beta", &[("beta.man.man", "b")]);

        let mut backend = new_backend();
        backend.open_distribution(root_a.to_str().unwrap()).unwrap();
        assert_eq!(backend.product_detail(1).unwrap().name, "alpha");

        backend.open_distribution(root_b.to_str().unwrap()).unwrap();
        assert_eq!(backend.product_detail(1).unwrap().name, "beta");
        assert_eq!(backend.image_detail(2).unwrap().name, "beta.man");
        assert_eq!(backend.subsystem_detail(3).unwrap().short_name, "man");
        assert!(
            backend
                .subsystem_detail(4)
                .err()
                .unwrap()
                .to_string()
                .contains("does not exist")
        );

        let _ = std::fs::remove_dir_all(&root_a);
        let _ = std::fs::remove_dir_all(&root_b);
    }

    /// Golden smoke test against a real distribution directory. Runs
    /// only when `SW_EXPLORER_TEST_DIST` points at one, mirroring the
    /// real-distribution tests of `sw-core`.
    #[test]
    fn real_dist_hierarchy_smoke() {
        let Some(raw) = std::env::var_os("SW_EXPLORER_TEST_DIST") else {
            eprintln!("SW_EXPLORER_TEST_DIST not set; skipping");
            return;
        };
        let path = PathBuf::from(raw);
        assert!(
            path.is_dir(),
            "SW_EXPLORER_TEST_DIST is set but not a directory: {}",
            path.display()
        );

        let mut backend = new_backend();
        backend.open_distribution(path.to_str().unwrap()).unwrap();
        let nodes = backend.hierarchy().unwrap();

        let products: Vec<_> = nodes
            .iter()
            .filter(|node| matches!(node.kind, ffi::ObjectKind::Product))
            .collect();
        assert!(
            products.len() >= 40,
            "expected a full media set, got {} products",
            products.len()
        );

        // Every parent reference must resolve to a node of the kind
        // the hierarchy demands.
        let by_id: std::collections::HashMap<u64, &ffi::HierarchyNode> =
            nodes.iter().map(|node| (node.id, node)).collect();
        assert_eq!(by_id.len(), nodes.len());
        for node in &nodes {
            match node.kind {
                ffi::ObjectKind::Product => assert_eq!(node.parent_id, 0),
                ffi::ObjectKind::Image => assert!(matches!(
                    by_id[&node.parent_id].kind,
                    ffi::ObjectKind::Product
                )),
                ffi::ObjectKind::Subsystem => assert!(matches!(
                    by_id[&node.parent_id].kind,
                    ffi::ObjectKind::Image
                )),
                _ => panic!("unexpected object kind"),
            }
        }

        // Entry counts must add up along the tree, and image names
        // must be the product name plus the image segment.
        for node in &nodes {
            let children = nodes.iter().filter(|child| child.parent_id == node.id);
            match node.kind {
                ffi::ObjectKind::Product | ffi::ObjectKind::Image => {
                    let sum: u64 = children.map(|child| child.entry_count).sum();
                    assert_eq!(
                        node.entry_count, sum,
                        "entry count mismatch at {}",
                        node.name
                    );
                }
                ffi::ObjectKind::Subsystem => {}
                _ => panic!("unexpected object kind"),
            }
            if matches!(node.kind, ffi::ObjectKind::Image) {
                let parent = by_id[&node.parent_id].name.to_string();
                assert!(
                    node.name.to_string().starts_with(&format!("{parent}.")),
                    "image name {} does not extend product name {parent}",
                    node.name
                );
            }
        }

        // A full media set has thousands of entries below eoe1/eoe-style
        // products; the sum over products is the distribution total.
        let total: u64 = products.iter().map(|node| node.entry_count).sum();
        assert!(total > 10_000, "suspiciously few entries: {total}");
    }

    /// Detail smoke test against a real distribution directory, using
    /// the same `SW_EXPLORER_TEST_DIST` opt-in as the hierarchy smoke
    /// test.
    #[test]
    fn real_dist_detail_smoke() {
        let Some(raw) = std::env::var_os("SW_EXPLORER_TEST_DIST") else {
            eprintln!("SW_EXPLORER_TEST_DIST not set; skipping");
            return;
        };
        let path = PathBuf::from(raw);
        assert!(
            path.is_dir(),
            "SW_EXPLORER_TEST_DIST is set but not a directory: {}",
            path.display()
        );

        let mut backend = new_backend();
        backend.open_distribution(path.to_str().unwrap()).unwrap();
        let nodes = backend.hierarchy().unwrap();

        // Every object id the hierarchy hands out must resolve through
        // its matching detail API, detail ids must round-trip, and
        // the presence flags must agree with the hierarchy node.
        for node in &nodes {
            match node.kind {
                ffi::ObjectKind::Product => {
                    let detail = backend.product_detail(node.id).unwrap();
                    assert_eq!(detail.id, node.id);
                    assert_eq!(detail.name, node.name);
                    assert_eq!(detail.descriptor_present, node.descriptor_present);
                    assert_eq!(detail.idb_present, node.idb_present);
                    assert_eq!(detail.entry_count, node.entry_count);
                }
                ffi::ObjectKind::Image => {
                    let detail = backend.image_detail(node.id).unwrap();
                    assert_eq!(detail.id, node.id);
                    assert_eq!(detail.name, node.name);
                    assert_eq!(detail.descriptor_present, node.descriptor_present);
                    assert_eq!(detail.idb_present, node.idb_present);
                    assert_eq!(detail.entry_count, node.entry_count);
                }
                ffi::ObjectKind::Subsystem => {
                    let detail = backend.subsystem_detail(node.id).unwrap();
                    assert_eq!(detail.id, node.id);
                    assert_eq!(detail.short_name, node.name);
                    assert_eq!(detail.descriptor_present, node.descriptor_present);
                    assert_eq!(detail.idb_present, node.idb_present);
                    assert_eq!(detail.entry_count, node.entry_count);
                    // "Known" tracks the descriptor record exactly:
                    // an IDB-only subsystem reports every
                    // descriptor-derived field as unknown, never as
                    // empty.
                    assert_eq!(detail.flags.known, detail.descriptor_present);
                    assert_eq!(detail.rules.known, detail.descriptor_present);
                    assert_eq!(detail.mach.known, detail.descriptor_present);
                    assert_eq!(detail.mapping_known, detail.descriptor_present);
                    assert_eq!(detail.autominiroot_known, detail.descriptor_present);
                }
                _ => panic!("unexpected object kind"),
            }
        }
    }
}
