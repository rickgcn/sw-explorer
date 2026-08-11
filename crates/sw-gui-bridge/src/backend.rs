//! The state behind the opaque CXX `Backend` handle: the loaded
//! distribution and the object-id table that hands out stable,
//! non-zero ids for every product, image and subsystem.
//!
//! Object-id invariants:
//!
//! * Object id 0 is never valid; real ids start at 1.
//! * Ids stay stable for the lifetime of one loaded distribution.
//! * Only a successful open replaces the distribution and its table;
//!   a failed open keeps the previous state untouched.
use crate::bridge::ffi;
use crate::detail;
use crate::entry;
use crate::extraction;
use crate::hardware;
use sw_core::distribution::Distribution;
use sw_core::idb::{Entry, EntryId};
use sw_core::mach::eval::HardwareProfile;
use sw_core::query::Query;

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
pub(crate) struct NoDistributionLoaded;

impl std::fmt::Display for NoDistributionLoaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no distribution loaded")
    }
}

impl std::error::Error for NoDistributionLoaded {}

/// Error for object detail lookups: no distribution loaded, or an
/// object id that is zero, unknown or of the wrong kind.
#[derive(Debug)]
pub(crate) struct ObjectDetailError(String);

impl std::fmt::Display for ObjectDetailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ObjectDetailError {}

/// Error for entry searches: no distribution loaded, or a query the
/// search policy rejects (an empty query is never a "match
/// everything" search).
#[derive(Debug)]
pub(crate) struct SearchError(String);

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SearchError {}

/// Error for hardware selection: no distribution loaded, or a
/// malformed profile pair (an empty attribute name).
#[derive(Debug)]
pub(crate) struct SelectionError(String);

impl std::fmt::Display for SelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SelectionError {}

/// Error for extraction planning and execution: no distribution
/// loaded, an invalid request (empty scope, duplicate or unresolvable
/// entry keys, a non-absolute output directory, an invalid `relative_to`
/// prefix, an empty hardware attribute name), or a planner refusal.
#[derive(Debug)]
pub(crate) struct ExtractionError(String);

impl std::fmt::Display for ExtractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExtractionError {}

/// Everything an extraction request resolves to: the concrete entries,
/// the validated options, the output directory and the hardware
/// profile.
struct ResolvedExtraction<'a> {
    /// Entries to extract, resolved against their owning products.
    entries: Vec<&'a Entry>,
    /// Validated core extraction options.
    options: sw_core::extract::ExtractOptions,
    /// Absolute output directory.
    out_dir: std::path::PathBuf,
    /// The hardware profile, or none.
    profile: Option<HardwareProfile>,
}

/// Rust-side backend state behind the opaque CXX handle.
pub(crate) struct Backend {
    pub(crate) distribution: Option<Distribution>,
    /// Object table of the loaded distribution: stable, non-zero ids
    /// for every product, image and subsystem. Rebuilt on every
    /// successful open; a failed open leaves it untouched.
    objects: Vec<ObjectEntry>,
}

pub(crate) fn new_backend() -> Box<Backend> {
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

    pub(crate) fn open_distribution(
        &mut self,
        path: &str,
    ) -> sw_core::error::Result<ffi::DistributionSummary> {
        // Open first, replace second: a failed open must not destroy
        // the distribution that is already loaded.
        let distribution = Distribution::open(path)?;
        let summary = ffi::DistributionSummary {
            product_count: distribution.products().len() as u64,
            diagnostic_count: distribution.diagnostic_count() as u64,
        };
        let objects = build_object_table(&distribution);
        self.distribution = Some(distribution);
        self.objects = objects;
        Ok(summary)
    }

    pub(crate) fn hierarchy(
        &self,
    ) -> std::result::Result<Vec<ffi::HierarchyNode>, NoDistributionLoaded> {
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

    pub(crate) fn product_detail(&self, id: u64) -> Result<ffi::ProductDetail, ObjectDetailError> {
        let (reference, distribution) = self.resolve_object(id)?;
        let ObjectRef::Product(product_index) = *reference else {
            return Err(wrong_kind(id, "a product", reference));
        };
        Ok(detail::product_detail(
            &distribution.products()[product_index],
            id,
        ))
    }

    pub(crate) fn image_detail(&self, id: u64) -> Result<ffi::ImageDetail, ObjectDetailError> {
        let (reference, distribution) = self.resolve_object(id)?;
        let ObjectRef::Image(product_index, image_index) = *reference else {
            return Err(wrong_kind(id, "an image", reference));
        };
        Ok(detail::image_detail(
            &distribution.products()[product_index].images[image_index],
            id,
        ))
    }

    pub(crate) fn subsystem_detail(
        &self,
        id: u64,
    ) -> Result<ffi::SubsystemDetail, ObjectDetailError> {
        let (reference, distribution) = self.resolve_object(id)?;
        let ObjectRef::Subsystem(product_index, image_index, subsystem_index) = *reference else {
            return Err(wrong_kind(id, "a subsystem", reference));
        };
        let image = &distribution.products()[product_index].images[image_index];
        Ok(detail::subsystem_detail(
            image,
            &image.subsystems[subsystem_index],
            id,
        ))
    }

    /// Object id of the product with the given index, found by
    /// scanning the object table — never derived from id arithmetic.
    fn product_object_id(&self, product_index: usize) -> Result<u64, ObjectDetailError> {
        self.objects
            .iter()
            .position(|entry| {
                matches!(entry.reference, ObjectRef::Product(index) if index == product_index)
            })
            .map(|position| position as u64 + 1)
            .ok_or_else(|| {
                ObjectDetailError(format!("product index {product_index} has no object id"))
            })
    }

    /// Maps every entry of the distribution to the object id of the
    /// product that actually owns it. Ownership is pointer identity
    /// into a product's entry list — never the product segment of the
    /// record's subsystem name: an IDB record may name a foreign
    /// subsystem and still belong to the product whose IDB carries
    /// it, and entry_detail resolves keys against that owner's entry
    /// list. The map is rebuilt per query and never cached.
    fn entry_owner_map(
        &self,
        distribution: &Distribution,
    ) -> Result<std::collections::HashMap<*const Entry, u64>, ObjectDetailError> {
        let mut owners = std::collections::HashMap::with_capacity(
            distribution
                .products()
                .iter()
                .map(|product| product.entries.len())
                .sum(),
        );
        for (product_index, product) in distribution.products().iter().enumerate() {
            let product_id = self.product_object_id(product_index)?;
            for entry in &product.entries {
                owners.insert(std::ptr::from_ref(entry), product_id);
            }
        }
        Ok(owners)
    }

    pub(crate) fn entries(
        &self,
        scope_id: u64,
    ) -> Result<Vec<ffi::EntrySummary>, ObjectDetailError> {
        let (reference, distribution) = self.resolve_object(scope_id)?;
        let product_index = match *reference {
            ObjectRef::Product(product_index) => product_index,
            ObjectRef::Image(product_index, _) => product_index,
            ObjectRef::Subsystem(product_index, ..) => product_index,
        };
        let product = &distribution.products()[product_index];
        let product_id = self.product_object_id(product_index)?;

        // Every scope filters the product's flat entry list, so the
        // result keeps the exact IDB order: interleaved subsystem
        // records are never regrouped, and duplicate paths are never
        // deduplicated.
        let scoped: Vec<&sw_core::idb::Entry> = match *reference {
            ObjectRef::Product(_) => product.entries.iter().collect(),
            ObjectRef::Image(_, image_index) => {
                let image_name = &product.images[image_index].name;
                product
                    .entries
                    .iter()
                    .filter(|entry| &entry.subsystem.image_name() == image_name)
                    .collect()
            }
            ObjectRef::Subsystem(_, image_index, subsystem_index) => {
                let name = &product.images[image_index].subsystems[subsystem_index].name;
                product
                    .entries
                    .iter()
                    .filter(|entry| &entry.subsystem == name)
                    .collect()
            }
        };
        Ok(scoped
            .into_iter()
            .map(|entry| entry::entry_summary(product_id, entry))
            .collect())
    }

    pub(crate) fn search_entries(
        &self,
        query: &str,
    ) -> Result<Vec<ffi::EntrySummary>, SearchError> {
        let distribution = self
            .distribution
            .as_ref()
            .ok_or_else(|| SearchError("no distribution loaded".to_string()))?;
        // An empty query means "no search", never "match everything":
        // a UI glitch must not dump the whole corpus.
        if query.is_empty() {
            return Err(SearchError("search query is empty".to_string()));
        }

        // The CLI query policy: plain text is a substring search; a
        // query carrying its own wildcard characters reaches the core
        // matcher unchanged.
        let pattern = if query.contains(['*', '?']) {
            query.to_string()
        } else {
            format!("*{query}*")
        };
        // Distribution::find yields hits in distribution product
        // order, each product in exact IDB order, and never
        // deduplicates duplicate paths.
        let found = distribution.find(&Query::path(pattern));

        // Maps each hit to the object id of the product that actually
        // owns the entry — the shared owner map, never the product
        // segment of the record's subsystem name.
        let owners = self
            .entry_owner_map(distribution)
            .map_err(|error| SearchError(error.to_string()))?;

        found
            .entries
            .iter()
            .map(|entry| {
                let product_id = owners
                    .get(&std::ptr::from_ref(*entry))
                    .ok_or_else(|| SearchError("search hit has no owning product".to_string()))?;
                Ok(entry::entry_summary(*product_id, entry))
            })
            .collect()
    }

    pub(crate) fn entry_detail(
        &self,
        product_id: u64,
        entry_id: u64,
    ) -> Result<ffi::EntryDetail, ObjectDetailError> {
        let (reference, distribution) = self.resolve_object(product_id)?;
        let ObjectRef::Product(product_index) = *reference else {
            return Err(wrong_kind(product_id, "a product", reference));
        };
        let product = &distribution.products()[product_index];
        let missing = || {
            ObjectDetailError(format!(
                "entry {entry_id} does not exist in product {}",
                product.name.as_str()
            ))
        };
        // Checked conversion first, then a bounds-checked lookup;
        // entry id 0 is a perfectly valid first entry.
        let index = usize::try_from(entry_id).map_err(|_| missing())?;
        let entry = product.entry(EntryId(index)).ok_or_else(missing)?;
        Ok(entry::entry_detail(product_id, entry))
    }

    pub(crate) fn hardware_candidates(
        &self,
    ) -> Result<Vec<ffi::HardwareCandidateSet>, NoDistributionLoaded> {
        let distribution = self.distribution.as_ref().ok_or(NoDistributionLoaded)?;
        Ok(hardware::hardware_candidates(distribution))
    }

    pub(crate) fn select_entries(
        &self,
        values: Vec<ffi::HardwareValue>,
    ) -> Result<ffi::SelectionSnapshot, SelectionError> {
        let distribution = self
            .distribution
            .as_ref()
            .ok_or_else(|| SelectionError("no distribution loaded".to_string()))?;

        // The profile carries hardware facts only; expression
        // semantics stay with the distribution metadata. Unknown
        // attribute names and values pass through unchanged, and an
        // empty value is a genuine fact (the media themselves carry
        // restrictions like `GFXBOARD=` for headless boards); only an
        // empty attribute name is malformed.
        let mut builder = HardwareProfile::builder();
        for pair in &values {
            if pair.attribute.is_empty() {
                return Err(SelectionError(
                    "hardware attribute name is empty".to_string(),
                ));
            }
            builder = builder.add(&pair.attribute, &pair.value);
        }
        let profile = builder.build();

        // All MACH semantics — hierarchy restrictions, entry-specific
        // expressions, mach-less fallback, duplicate paths, unresolved
        // expressions and conflicts — come from the core selection.
        let selection = distribution.select(&profile);
        let owners = self
            .entry_owner_map(distribution)
            .map_err(|error| SelectionError(error.to_string()))?;
        hardware::selection_snapshot(&selection, &owners).map_err(SelectionError)
    }

    /// Resolves the entry keys of an extraction request against the
    /// loaded distribution. Each key resolves directly in the product
    /// its object id identifies — the owning product, never a product
    /// guessed from the record's subsystem name. Exact duplicates are
    /// rejected; the same path under different keys is legitimate.
    fn resolve_entries<'a>(
        &self,
        distribution: &'a Distribution,
        request: &ffi::ExtractionRequest,
    ) -> Result<Vec<&'a Entry>, ExtractionError> {
        if request.entries.is_empty() {
            return Err(ExtractionError("no entries requested".to_string()));
        }
        let mut seen = std::collections::HashSet::new();
        let mut entries = Vec::with_capacity(request.entries.len());
        for key in &request.entries {
            if !seen.insert((key.product_id, key.entry_id)) {
                return Err(ExtractionError(format!(
                    "duplicate entry key {}:{}",
                    key.product_id, key.entry_id
                )));
            }
            let (reference, _) = self
                .resolve_object(key.product_id)
                .map_err(|error| ExtractionError(error.to_string()))?;
            let ObjectRef::Product(product_index) = *reference else {
                return Err(ExtractionError(format!(
                    "object {} does not identify a product",
                    key.product_id
                )));
            };
            let product = &distribution.products()[product_index];
            // Checked conversion first, then a bounds-checked lookup;
            // entry id 0 is a perfectly valid first entry.
            let index = usize::try_from(key.entry_id).map_err(|_| {
                ExtractionError(format!(
                    "entry id {} does not exist in product {}",
                    key.entry_id,
                    product.name.as_str()
                ))
            })?;
            let entry = product.entry(EntryId(index)).ok_or_else(|| {
                ExtractionError(format!(
                    "entry id {} does not exist in product {}",
                    key.entry_id,
                    product.name.as_str()
                ))
            })?;
            entries.push(entry);
        }
        Ok(entries)
    }

    /// Everything an extraction request resolves to: the concrete
    /// entries, the validated options, the output directory and the
    /// hardware profile.
    fn resolve_extraction<'a>(
        &self,
        distribution: &'a Distribution,
        request: &ffi::ExtractionRequest,
    ) -> Result<ResolvedExtraction<'a>, ExtractionError> {
        let entries = self.resolve_entries(distribution, request)?;
        let (options, out_dir) = extraction::extract_options(request).map_err(ExtractionError)?;
        let profile = extraction::hardware_profile(request).map_err(ExtractionError)?;
        Ok(ResolvedExtraction {
            entries,
            options,
            out_dir,
            profile,
        })
    }

    pub(crate) fn plan_extraction(
        &self,
        request: &ffi::ExtractionRequest,
    ) -> Result<ffi::ExtractionPlanSummary, ExtractionError> {
        let distribution = self
            .distribution
            .as_ref()
            .ok_or_else(|| ExtractionError("no distribution loaded".to_string()))?;
        let resolved = self.resolve_extraction(distribution, request)?;
        // All safety semantics come from the shared core planner; the
        // bridge only resolves keys and converts the summary.
        let plan = sw_core::plan::plan_extraction(
            distribution,
            &resolved.entries,
            &resolved.out_dir,
            &resolved.options,
            resolved.profile.as_ref(),
        )
        .map_err(|error| ExtractionError(error.to_string()))?;
        Ok(extraction::plan_summary(plan.summary))
    }

    pub(crate) fn extract_entries(
        &self,
        request: &ffi::ExtractionRequest,
    ) -> Result<ffi::ExtractionReportDetail, ExtractionError> {
        let distribution = self
            .distribution
            .as_ref()
            .ok_or_else(|| ExtractionError("no distribution loaded".to_string()))?;
        let resolved = self.resolve_extraction(distribution, request)?;
        // Checked execution re-plans immediately before writing; a
        // refusal means zero writes, and individual runtime failures
        // arrive as report data.
        let checked = sw_core::plan::extract_checked(
            distribution,
            &resolved.entries,
            &resolved.out_dir,
            &resolved.options,
            resolved.profile.as_ref(),
        )
        .map_err(|error| ExtractionError(error.to_string()))?;
        Ok(extraction::report_detail(checked.report))
    }
}
