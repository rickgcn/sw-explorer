//! Tests for the bridge: object-id behaviour, hierarchy flattening,
//! detail DTO semantics, and opt-in smoke tests against a real
//! distribution.
use crate::backend::new_backend;
use crate::bridge::ffi;
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
    fn plain(name: &'static str, title: &'static str, mapping: &'static str, flags: u16) -> Self {
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
