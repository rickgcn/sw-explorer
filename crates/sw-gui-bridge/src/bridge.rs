//! The CXX bridge itself: an opaque [`Backend`] handle plus the plain
//! data structs returned across the boundary.
//!
//! All `sw-core` errors are propagated to C++ as CXX exceptions; the
//! bridge contains no business logic of its own.
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
    }
}

/// Identifies one domain object of the loaded distribution by its
/// position: product index, image index and subsystem index into
/// `Distribution::products()` and below.
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

impl Backend {
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
}
