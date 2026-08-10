//! The CXX ABI declaration: the complete schema of what Rust exposes
//! to the Qt/C++ frontend, kept in a single `#[cxx::bridge]` module
//! so the whole boundary can be reviewed at a glance.
//!
//! All `sw-core` errors are propagated to C++ as CXX exceptions; the
//! bridge declares no business logic of its own. The `extern "Rust"`
//! items are implemented in `backend` (the opaque `Backend` handle,
//! object ids, hierarchy) and `detail` (detail DTO construction).
use crate::backend::{Backend, new_backend};

#[cxx::bridge(namespace = "sw")]
pub(crate) mod ffi {
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

    /// The kind of filesystem object an IDB record describes.
    ///
    /// Mirrors `sw_core::idb::FileType`; a type letter the bridge does
    /// not know maps to `Other`, with the raw letter preserved in
    /// `file_type_raw`.
    enum EntryFileType {
        /// `f` — a regular file.
        Regular,
        /// `d` — a directory.
        Directory,
        /// `l` — a symbolic link.
        SymbolicLink,
        /// `b` — a block special device.
        BlockDevice,
        /// `c` — a character special device.
        CharacterDevice,
        /// `p` — a named pipe (FIFO).
        Fifo,
        /// Any other type letter; `file_type_raw` carries it.
        Other,
    }

    /// One row of the entry browser: one IDB record.
    ///
    /// Rows are IDB records, not a deduplicated installed filesystem:
    /// the same path may appear several times (hardware-conditional
    /// variants) and every occurrence is its own row.
    struct EntrySummary {
        /// Object id of the containing product.
        product_id: u64,
        /// The sw-core entry id: the 0-based position in the
        /// product's entry list. Zero is a perfectly valid id.
        entry_id: u64,

        /// Normalized install path, e.g. `usr/bin/Xsgi`.
        path: String,
        /// Fully qualified subsystem, e.g. `eoe.sw.unix`.
        subsystem: String,

        /// The kind of filesystem object.
        file_type: EntryFileType,
        /// The raw IDB type letter; meaningful even when `file_type`
        /// is `Other`.
        file_type_raw: String,

        /// Whether the `size(...)` attribute is present.
        size_known: bool,
        /// Uncompressed size in bytes, when known.
        size: u64,

        /// Whether the archive stored size is known.
        stored_size_known: bool,
        /// Bytes occupied in the image archive: `cmpsize` when
        /// non-zero, `size` when stored uncompressed. A `cmpsize(0)`
        /// record never surfaces as "stored size 0".
        stored_size: u64,

        /// Parsed `mach(...)` expression texts, exactly as written.
        mach: Vec<String>,
        /// Raw `mach(...)` payloads that could not be parsed; never
        /// dropped.
        unresolved_mach: Vec<String>,
    }

    /// Everything the inspector shows for one IDB entry.
    ///
    /// The important attributes are structured; the complete original
    /// IDB record line is kept as the lossless fallback for every
    /// attribute that has no dedicated field.
    struct EntryDetail {
        /// Object id of the containing product.
        product_id: u64,
        /// The sw-core entry id; zero is a perfectly valid id.
        entry_id: u64,

        /// The kind of filesystem object.
        file_type: EntryFileType,
        /// The raw IDB type letter; meaningful even when `file_type`
        /// is `Other`.
        file_type_raw: String,

        /// Permission bits, parsed from the octal IDB field.
        mode: u32,
        /// Owner name, e.g. `root`.
        owner: String,
        /// Group name, e.g. `sys`.
        group: String,

        /// Normalized install path.
        path: String,
        /// Install path exactly as written in the IDB.
        raw_path: String,
        /// Build-tree source path recorded by `gendist`.
        source_path: String,
        /// Fully qualified subsystem, e.g. `eoe.sw.unix`.
        subsystem: String,

        /// Whether the `size(...)` attribute is present.
        size_known: bool,
        /// Uncompressed size in bytes, when known.
        size: u64,

        /// Whether the `cmpsize(...)` attribute is present.
        compressed_size_known: bool,
        /// Size as stored in the image archive, when known; zero
        /// means the file is stored uncompressed.
        compressed_size: u64,

        /// Whether the archive stored size is known.
        stored_size_known: bool,
        /// Bytes occupied in the image archive (`cmpsize` when
        /// non-zero, `size` when stored uncompressed).
        stored_size: u64,

        /// Whether the `sum(...)` checksum attribute is present.
        checksum_known: bool,
        /// Checksum of the installed file, when known.
        checksum: u64,

        /// Whether the `config(...)` attribute is present.
        config_known: bool,
        /// Config mode: `suggest`, `update`, `noupdate`, or the raw
        /// value when it is none of the known modes.
        config_mode: String,

        /// Whether the entry carries a `symval(...)` target.
        symlink_target_known: bool,
        /// Symbolic link target, when known.
        symlink_target: String,

        /// Whether the entry carries `dev(major minor)` numbers.
        device_known: bool,
        /// Major device number, when known.
        device_major: u32,
        /// Minor device number, when known.
        device_minor: u32,

        /// Parsed `mach(...)` expression texts, exactly as written.
        /// An entry without MACH attributes is known to be
        /// unrestricted, which is not the same as unknown.
        mach: Vec<String>,
        /// Raw `mach(...)` payloads that could not be parsed; never
        /// dropped.
        unresolved_mach: Vec<String>,

        /// Whether the entry carries payload bytes in an image
        /// archive.
        payload_present: bool,
        /// The image archive holding the payload, e.g. `eoe.sw`.
        payload_image: String,

        /// Whether the stored payload size is known.
        payload_encoded_size_known: bool,
        /// Size of the payload as stored, when known.
        payload_encoded_size: u64,

        /// Whether the layout algorithm could predict the record
        /// offset. This is an *expected* offset: it has not been
        /// confirmed by reading the archive.
        expected_record_offset_known: bool,
        /// Expected record offset, when known.
        expected_record_offset: u64,

        /// The IDB file the entry was parsed from.
        origin_idb_path: String,
        /// 1-based line number within that file.
        origin_line: u64,
        /// The complete original IDB record line.
        raw_idb_line: String,
    }

    /// One hardware attribute/value pair of a hardware profile: a fact
    /// about the simulated target machine, e.g. `CPUBOARD=IP22`. The
    /// same attribute may appear several times with different values.
    struct HardwareValue {
        /// The hardware attribute name, e.g. `CPUBOARD`. Unknown
        /// names are passed through unchanged.
        attribute: String,
        /// The attribute value, e.g. `IP22`. May be empty: the media
        /// carry restrictions like `GFXBOARD=`.
        value: String,
    }

    /// The candidate values of one hardware attribute, discovered in
    /// the loaded distribution's parsed MACH expressions.
    struct HardwareCandidateSet {
        /// The hardware attribute name, e.g. `CPUBOARD`; unknown
        /// attribute names are preserved verbatim.
        attribute: String,
        /// Values the distribution's MACH expressions compare this
        /// attribute against, deduplicated, in first-appearance order.
        values: Vec<String>,
    }

    /// The backend identity of one selected entry: the object id of
    /// the product whose entry list actually holds the record, plus
    /// the sw-core entry id inside that product.
    struct SelectionEntryKey {
        /// Object id of the owning product.
        product_id: u64,
        /// The sw-core entry id; zero is a perfectly valid id.
        entry_id: u64,
    }

    /// One contested path plus the entries competing for it, exactly
    /// as the core selection reports it.
    struct SelectionConflictDetail {
        /// The contested path.
        path: String,
        /// The competing candidates; never resolved by the bridge.
        candidates: Vec<SelectionEntryKey>,
    }

    /// Which entries a hardware profile selects across the whole
    /// distribution. Selection carries entry identities only; the
    /// entry list and inspector data keep coming from `entries`,
    /// `search_entries` and `entry_detail`.
    struct SelectionSnapshot {
        /// Entries that would be installed, in distribution order.
        /// Conflict candidates stay included.
        selected: Vec<SelectionEntryKey>,
        /// Paths with ambiguous or unresolvable candidates.
        conflicts: Vec<SelectionConflictDetail>,
    }

    /// How entry paths are mapped below the output directory.
    enum ExtractionPathMode {
        /// Rebuild the full IRIX hierarchy.
        Full,
        /// Write every entry directly into the output directory.
        Flat,
        /// Strip the `relative_to` prefix from entry paths.
        RelativeTo,
    }

    /// Whether compressed payloads are decoded during extraction.
    enum ExtractionDecodeMode {
        /// Decode `.Z` payloads to their plain contents.
        Auto,
        /// Write payloads exactly as stored.
        Never,
    }

    /// The backend identity of one entry to extract: the object id of
    /// the product whose entry list actually holds the record, plus the
    /// sw-core entry id inside that product. Never re-derived from a
    /// row, a path or a subsystem name.
    struct ExtractionEntryKey {
        /// Object id of the owning product.
        product_id: u64,
        /// The sw-core entry id; zero is a perfectly valid id.
        entry_id: u64,
    }

    /// Everything one extraction needs, for both preflight planning and
    /// execution. The scope arrives as concrete entry keys — the bridge
    /// knows nothing about hierarchy scopes, search modes or selection
    /// overlays.
    struct ExtractionRequest {
        /// The entries to extract, in display order.
        entries: Vec<ExtractionEntryKey>,
        /// The hardware profile to apply, as attribute/value pairs; an
        /// empty list means no profile. An empty attribute name is
        /// rejected; an empty value is a genuine fact (`GFXBOARD=`).
        hardware: Vec<HardwareValue>,
        /// The output directory; must be an absolute host path and need
        /// not exist yet.
        output_dir: String,
        /// How entry paths are mapped below the output directory.
        path_mode: ExtractionPathMode,
        /// The prefix for `RelativeTo`; ignored by the other modes.
        relative_to: String,
        /// Whether compressed payloads are decoded.
        decode: ExtractionDecodeMode,
        /// With `Auto`, additionally write the stored `.Z` bytes.
        keep_stored: bool,
        /// Continue past individual failures instead of aborting.
        continue_on_error: bool,
        /// Allow overwriting existing regular files. Symbolic links,
        /// directories and special files are never treated as
        /// overwriteable regular files regardless of this flag.
        allow_overwrite: bool,
    }

    /// Counts describing a confirmed extraction plan, for user
    /// confirmation. The planned entries themselves never cross the
    /// bridge; execution always re-plans from the request.
    struct ExtractionPlanSummary {
        /// Entries the frontend handed to the planner.
        requested_records: u64,
        /// Entries that deliberately produce no output (path-mode
        /// exclusions, devices, FIFOs, symbolic links without a
        /// recorded target).
        omitted_records: u64,
        /// Entries that would produce output but were excluded by the
        /// hardware profile selection. Zero without a profile.
        hardware_excluded_records: u64,
        /// Entries that will be handed to the extraction.
        planned_records: u64,
        /// Projected output paths of the planned entries.
        output_paths: u64,
        /// Existing regular files that will be overwritten with
        /// `allow_overwrite`.
        existing_outputs: u64,
    }

    /// A single entry that failed to extract.
    struct ExtractionFailureDetail {
        /// Entry path.
        path: String,
        /// What went wrong.
        message: String,
    }

    /// How a payload record was located when it was not at its expected
    /// offset. `Exact` never appears here: exact reads are not
    /// recoveries.
    enum ExtractionRecoveryKind {
        /// Found at a known offset delta.
        Delta,
        /// Found by resynchronizing the archive walk.
        Resynced,
        /// Found by scanning the archive.
        Scanned,
    }

    /// A payload that was not found exactly at its expected offset: an
    /// honest success the user must see, not a silent one.
    struct ExtractionRecoveryDetail {
        /// Entry path.
        path: String,
        /// Whether an expected record offset is known.
        expected_known: bool,
        /// Expected record offset; valid only when `expected_known`.
        expected_offset: u64,
        /// Offset the record was actually found at.
        actual_offset: u64,
        /// How the record was located.
        kind: ExtractionRecoveryKind,
        /// Whether a delta is known.
        delta_known: bool,
        /// Offset delta; valid only when `delta_known`.
        delta: i64,
    }

    /// The outcome of an executed extraction. Individual failures are
    /// data, not errors: a refusal (the plan rejected the extraction
    /// before any write) is the method error instead.
    struct ExtractionReportDetail {
        /// Entries successfully written.
        extracted: u64,
        /// Entries deliberately not written.
        skipped: u64,
        /// Entries that failed.
        failures: Vec<ExtractionFailureDetail>,
        /// Payloads that needed recovery to be located.
        recoveries: Vec<ExtractionRecoveryDetail>,
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

        /// Lists the IDB entries below a product, image or subsystem
        /// scope, in exact IDB order.
        ///
        /// Every scope filters the product's flat entry list, so
        /// interleaved subsystem records are never regrouped and
        /// duplicate paths (hardware-conditional variants) are never
        /// deduplicated.
        ///
        /// Fails when no distribution is loaded or when `scope_id`
        /// is zero or unknown.
        fn entries(self: &Backend, scope_id: u64) -> Result<Vec<EntrySummary>>;

        /// Searches the whole distribution for entries whose path
        /// matches `query`, across every product, image and
        /// subsystem. Rows come back in distribution product order,
        /// each product in exact IDB order; duplicate paths
        /// (hardware-conditional variants) are never deduplicated.
        ///
        /// Query policy, identical to the CLI: plain text is a
        /// substring search (`foo` matches as `*foo*`); a query
        /// containing `*` or `?` is handed to the core wildcard
        /// matcher unchanged.
        ///
        /// Every row carries the object id of the product that owns
        /// the entry — the product whose entry list holds the record,
        /// which may name a foreign subsystem — so `entry_detail`
        /// can resolve it directly.
        ///
        /// Fails when no distribution is loaded or when `query` is
        /// empty; an empty query is never a "match everything"
        /// search.
        fn search_entries(self: &Backend, query: &str) -> Result<Vec<EntrySummary>>;

        /// Returns the detail snapshot of one entry.
        ///
        /// `product_id` must identify a product and `entry_id` must
        /// exist in that product's entry list; entry id 0 is valid.
        ///
        /// Fails when no distribution is loaded, when `product_id`
        /// is zero, unknown or not a product, or when `entry_id`
        /// is out of range.
        fn entry_detail(self: &Backend, product_id: u64, entry_id: u64) -> Result<EntryDetail>;

        /// Lists the hardware attribute values found in the loaded
        /// distribution's parsed MACH expressions (product, image and
        /// subsystem restrictions plus entry `mach(...)` attributes),
        /// grouped by attribute, deduplicated, in first-appearance
        /// order. Unparseable expression payloads are never guessed
        /// at and contribute nothing. Unknown attribute names are
        /// preserved verbatim.
        ///
        /// Fails when no distribution is loaded.
        fn hardware_candidates(self: &Backend) -> Result<Vec<HardwareCandidateSet>>;

        /// Computes which entries the given hardware profile selects,
        /// across the whole distribution. All MACH semantics — product,
        /// image, subsystem and entry expressions, mach-less fallback,
        /// duplicate paths, unparseable expressions and conflicts —
        /// come from `sw-core`; the bridge only translates entry
        /// references into entry keys.
        ///
        /// Each pair is one attribute value; an attribute may occur
        /// several times. An empty attribute name is rejected; an
        /// empty value is a genuine fact (the media carry
        /// restrictions like `GFXBOARD=`). Unknown attributes and
        /// values are passed through unchanged.
        ///
        /// Fails when no distribution is loaded.
        fn select_entries(self: &Backend, values: Vec<HardwareValue>) -> Result<SelectionSnapshot>;

        /// Plans an extraction without touching the filesystem.
        ///
        /// Every check — hardware selection, ambiguity, output
        /// collisions, output topology, existing ancestors and existing
        /// outputs — comes from the shared `sw-core` planner; the
        /// bridge only resolves entry keys and converts the summary.
        /// Planning never creates or modifies anything, including the
        /// output directory itself.
        ///
        /// Fails when no distribution is loaded; when the request has
        /// no entries, a duplicate entry key, a key whose product id is
        /// zero, unknown or not a product, or an entry id out of range;
        /// when the output directory is empty or not absolute; when the
        /// `relative_to` prefix is not a valid IRIX path; when a
        /// hardware attribute name is empty; or when the planner
        /// refuses the extraction.
        fn plan_extraction(
            self: &Backend,
            request: &ExtractionRequest,
        ) -> Result<ExtractionPlanSummary>;

        /// Executes an extraction.
        ///
        /// The plan is recomputed inside, immediately before writing;
        /// a refusal here means zero files were written by this call.
        /// Runtime failures of individual entries are reported through
        /// the returned report, not as errors.
        ///
        /// Fails under the same conditions as `plan_extraction`.
        fn extract_entries(
            self: &Backend,
            request: &ExtractionRequest,
        ) -> Result<ExtractionReportDetail>;
    }
}
