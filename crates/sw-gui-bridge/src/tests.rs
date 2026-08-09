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

/// Writes an IDB file with the exact given content, for entry tests
/// that need full control over every record.
fn write_raw_idb(root: &Path, product: &str, content: &str) {
    std::fs::write(root.join(format!("{product}.idb")), content).unwrap();
}

/// Writes the standard entry test distribution: one IDB-only product
/// `entries` whose records exercise ordering, duplicate paths, file
/// types, MACH variants and size semantics.
///
/// Image `entries.sw` holds subsystems `alpha` and `beta` with
/// deliberately interleaved records; image `entries.man` holds `man`.
///
/// Object ids: 1 = product `entries`, 2 = image `entries.sw`,
/// 3 = `alpha`, 4 = `beta`, 5 = image `entries.man`, 6 = `man`.
fn write_entries_dist(root: &Path) {
    write_raw_idb(
        root,
        "entries",
        "f 0755 root sys usr/bin/a src/a entries.sw.alpha sum(10) size(100) cmpsize(60)\n\
         f 0644 root sys usr/lib/libx.so src/libx entries.sw.beta sum(20) size(200) cmpsize(0) mach(CPUBOARD=IP22)\n\
         f 0644 root sys usr/lib/libx.so src/libx2 entries.sw.beta sum(21) size(210) cmpsize(0) mach(CPUBOARD=IP26)\n\
         d 0755 root sys etc src/etc entries.sw.alpha\n\
         f 0644 root sys man/a src/man/a entries.man.man sum(30) size(300) cmpsize(0)\n\
         l 0777 root sys usr/bin/link src/l entries.sw.alpha symval(../lib/libx.so)\n\
         b 0600 root sys dev/dsk0 src/d entries.sw.alpha dev(1 2)\n\
         f 0644 root sys usr/lib/libx.so src/libx3 entries.sw.beta sum(22) size(220) cmpsize(50) mach(=GARBAGE)\n\
         x 0644 root sys weird src/w entries.sw.alpha sum(1) size(5) cmpsize(0)\n\
         f 0644 root sys usr/bin/cfg src/cfg entries.sw.alpha sum(9) size(9) cmpsize(0) config(wildmode)\n\
         f 0644 root sys usr/bin/nocmp src/n entries.sw.alpha sum(5) size(77)\n\
         f 0644 root sys man/b src/man/b entries.man.man sum(31) size(310) cmpsize(0)\n",
    );
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

#[test]
fn entries_require_loaded_distribution() {
    let backend = new_backend();
    assert_eq!(
        backend.entries(1).err().unwrap().to_string(),
        "no distribution loaded"
    );
    assert_eq!(
        backend.entry_detail(1, 0).err().unwrap().to_string(),
        "no distribution loaded"
    );
}

#[test]
fn entries_reject_zero_and_out_of_range_scope_ids() {
    let root = temp_root("entries-bad-scope");
    write_entries_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    // Six objects: product, two images with their subsystems.
    assert_eq!(backend.hierarchy().unwrap().len(), 6);

    assert_eq!(
        backend.entries(0).err().unwrap().to_string(),
        "object id 0 does not identify a distribution object"
    );
    assert_eq!(
        backend.entries(7).err().unwrap().to_string(),
        "object 7 does not exist"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn product_scope_returns_all_entries_in_idb_order() {
    let root = temp_root("entries-product-scope");
    write_entries_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    let rows = backend.entries(1).unwrap();
    assert_eq!(rows.len(), 12);
    // Product scope is the flat entry list: ids are exactly 0..n and
    // every row carries the product's object id.
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.entry_id, index as u64);
        assert_eq!(row.product_id, 1);
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn image_scope_keeps_interleaved_idb_order() {
    let root = temp_root("entries-image-scope");
    write_entries_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // Image entries.sw (object 2) holds alpha and beta records that
    // are interleaved in the IDB; the scope must not regroup them
    // by subsystem.
    let rows = backend.entries(2).unwrap();
    let paths: Vec<&str> = rows.iter().map(|row| row.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "usr/bin/a",
            "usr/lib/libx.so",
            "usr/lib/libx.so",
            "etc",
            "usr/bin/link",
            "dev/dsk0",
            "usr/lib/libx.so",
            "weird",
            "usr/bin/cfg",
            "usr/bin/nocmp",
        ]
    );
    let ids: Vec<u64> = rows.iter().map(|row| row.entry_id).collect();
    assert_eq!(ids, vec![0, 1, 2, 3, 5, 6, 7, 8, 9, 10]);
    assert!(rows.iter().all(|row| row.product_id == 1));

    // The other image sees only its own records.
    let man = backend.entries(5).unwrap();
    let man_paths: Vec<&str> = man.iter().map(|row| row.path.as_str()).collect();
    assert_eq!(man_paths, vec!["man/a", "man/b"]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn subsystem_scope_keeps_idb_order_and_duplicate_paths() {
    let root = temp_root("entries-subsystem-scope");
    write_entries_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // Subsystem beta (object 4): three records, all with the same
    // path. They are IDB records — hardware-conditional variants —
    // and must stay three rows in their original relative order.
    let beta = backend.entries(4).unwrap();
    assert_eq!(beta.len(), 3);
    let ids: Vec<u64> = beta.iter().map(|row| row.entry_id).collect();
    assert_eq!(ids, vec![1, 2, 7]);
    assert!(beta.iter().all(|row| row.path == "usr/lib/libx.so"));
    assert!(
        beta.iter()
            .all(|row| row.subsystem == "entries.sw.beta" && row.product_id == 1)
    );

    let alpha = backend.entries(3).unwrap();
    let alpha_ids: Vec<u64> = alpha.iter().map(|row| row.entry_id).collect();
    assert_eq!(alpha_ids, vec![0, 3, 5, 6, 8, 9, 10]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn entry_summary_reports_sizes_types_and_mach() {
    let root = temp_root("entries-summary-fields");
    write_entries_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    let rows = backend.entries(1).unwrap();

    // Compressed file: stored size is cmpsize, not size.
    assert!(rows[0].size_known);
    assert_eq!(rows[0].size, 100);
    assert!(rows[0].stored_size_known);
    assert_eq!(rows[0].stored_size, 60);
    assert!(matches!(rows[0].file_type, ffi::EntryFileType::Regular));
    assert_eq!(rows[0].file_type_raw, "f");

    // cmpsize(0) means stored uncompressed: stored size is the plain
    // size, never 0.
    assert!(rows[1].stored_size_known);
    assert_eq!(rows[1].stored_size, 200);
    assert_eq!(rows[1].mach, vec!["CPUBOARD=IP22".to_string()]);
    assert!(rows[1].unresolved_mach.is_empty());

    // Directory: no size attributes at all, both unknown.
    assert!(!rows[3].size_known);
    assert!(!rows[3].stored_size_known);
    assert!(matches!(rows[3].file_type, ffi::EntryFileType::Directory));
    assert_eq!(rows[3].file_type_raw, "d");

    // Unresolved MACH is kept, never dropped.
    assert!(rows[7].mach.is_empty());
    assert_eq!(rows[7].unresolved_mach, vec!["=GARBAGE".to_string()]);
    assert_eq!(rows[7].stored_size, 50);

    // Unknown type letter: enum maps to Other, raw letter preserved.
    assert!(matches!(rows[8].file_type, ffi::EntryFileType::Other));
    assert_eq!(rows[8].file_type_raw, "x");

    // No cmpsize at all: stored size unknown.
    assert!(rows[10].size_known);
    assert_eq!(rows[10].size, 77);
    assert!(!rows[10].stored_size_known);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn entry_detail_decodes_regular_file() {
    let root = temp_root("entry-detail-regular");
    write_entries_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // Entry id 0 is a perfectly valid id.
    let detail = backend.entry_detail(1, 0).unwrap();
    assert_eq!(detail.product_id, 1);
    assert_eq!(detail.entry_id, 0);
    assert!(matches!(detail.file_type, ffi::EntryFileType::Regular));
    assert_eq!(detail.file_type_raw, "f");
    assert_eq!(detail.mode, 0o755);
    assert_eq!(detail.owner, "root");
    assert_eq!(detail.group, "sys");
    assert_eq!(detail.path, "usr/bin/a");
    assert_eq!(detail.raw_path, "usr/bin/a");
    assert_eq!(detail.source_path, "src/a");
    assert_eq!(detail.subsystem, "entries.sw.alpha");
    assert!(detail.size_known);
    assert_eq!(detail.size, 100);
    assert!(detail.compressed_size_known);
    assert_eq!(detail.compressed_size, 60);
    assert!(detail.stored_size_known);
    assert_eq!(detail.stored_size, 60);
    assert!(detail.checksum_known);
    assert_eq!(detail.checksum, 10);
    assert!(!detail.config_known);
    assert!(!detail.symlink_target_known);
    assert!(!detail.device_known);
    assert!(detail.mach.is_empty());
    assert!(detail.unresolved_mach.is_empty());

    // Payload locator metadata: the entry is the first payload-bearing
    // record of its image, so the layout algorithm predicts the offset
    // right behind the 13-byte archive header.
    assert!(detail.payload_present);
    assert_eq!(detail.payload_image, "entries.sw");
    assert!(detail.payload_encoded_size_known);
    assert_eq!(detail.payload_encoded_size, 60);
    assert!(detail.expected_record_offset_known);
    assert_eq!(detail.expected_record_offset, 13);

    assert!(detail.origin_idb_path.ends_with("entries.idb"));
    assert_eq!(detail.origin_line, 1);
    assert_eq!(
        detail.raw_idb_line,
        "f 0755 root sys usr/bin/a src/a entries.sw.alpha sum(10) size(100) cmpsize(60)"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn entry_detail_decodes_special_file_kinds() {
    let root = temp_root("entry-detail-kinds");
    write_entries_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    let link = backend.entry_detail(1, 5).unwrap();
    assert!(matches!(link.file_type, ffi::EntryFileType::SymbolicLink));
    assert_eq!(link.file_type_raw, "l");
    assert!(link.symlink_target_known);
    assert_eq!(link.symlink_target, "../lib/libx.so");
    // No size attributes: sizes and payload are unknown, not zero.
    assert!(!link.size_known);
    assert!(!link.stored_size_known);
    assert!(!link.checksum_known);
    assert!(!link.payload_present);

    let device = backend.entry_detail(1, 6).unwrap();
    assert!(matches!(device.file_type, ffi::EntryFileType::BlockDevice));
    assert_eq!(device.file_type_raw, "b");
    assert_eq!(device.mode, 0o600);
    assert!(device.device_known);
    assert_eq!(device.device_major, 1);
    assert_eq!(device.device_minor, 2);

    // Unknown config value is preserved verbatim.
    let config = backend.entry_detail(1, 9).unwrap();
    assert!(config.config_known);
    assert_eq!(config.config_mode, "wildmode");

    // Unknown type letter survives in the detail as well.
    let weird = backend.entry_detail(1, 8).unwrap();
    assert!(matches!(weird.file_type, ffi::EntryFileType::Other));
    assert_eq!(weird.file_type_raw, "x");

    // Entry without cmpsize carries no payload locator.
    let uncompressed = backend.entry_detail(1, 10).unwrap();
    assert!(!uncompressed.compressed_size_known);
    assert!(!uncompressed.stored_size_known);
    assert!(!uncompressed.payload_present);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn entry_detail_keeps_unresolved_mach_and_origin() {
    let root = temp_root("entry-detail-mach");
    write_entries_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    let detail = backend.entry_detail(1, 7).unwrap();
    assert!(detail.mach.is_empty());
    assert_eq!(detail.unresolved_mach, vec!["=GARBAGE".to_string()]);
    assert_eq!(detail.origin_line, 8);
    assert_eq!(
        detail.raw_idb_line,
        "f 0644 root sys usr/lib/libx.so src/libx3 entries.sw.beta sum(22) size(220) cmpsize(50) mach(=GARBAGE)"
    );

    let parsed = backend.entry_detail(1, 1).unwrap();
    assert_eq!(parsed.mach, vec!["CPUBOARD=IP22".to_string()]);
    assert!(parsed.unresolved_mach.is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn entry_detail_rejects_bad_keys() {
    let root = temp_root("entry-detail-bad-keys");
    write_entries_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    assert_eq!(
        backend.entry_detail(0, 0).err().unwrap().to_string(),
        "object id 0 does not identify a distribution object"
    );
    // The product id must actually identify a product.
    assert_eq!(
        backend.entry_detail(2, 0).err().unwrap().to_string(),
        "object 2 is an image, not a product"
    );
    assert_eq!(
        backend.entry_detail(3, 0).err().unwrap().to_string(),
        "object 3 is a subsystem, not a product"
    );
    assert_eq!(
        backend.entry_detail(1000, 0).err().unwrap().to_string(),
        "object 1000 does not exist"
    );
    // Out-of-range entry ids fail with the product named, including
    // ids that do not even fit the platform's usize.
    assert_eq!(
        backend.entry_detail(1, 12).err().unwrap().to_string(),
        "entry 12 does not exist in product entries"
    );
    assert_eq!(
        backend.entry_detail(1, u64::MAX).err().unwrap().to_string(),
        "entry 18446744073709551615 does not exist in product entries"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn failed_open_keeps_previous_entry_keys_valid() {
    let root = temp_root("entries-keep");
    write_entries_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    let before = backend.entry_detail(1, 0).unwrap();

    let missing = root.join("missing");
    assert!(
        backend
            .open_distribution(missing.to_str().unwrap())
            .is_err()
    );

    // The failed open touched nothing: the old entry keys still work.
    assert_eq!(backend.entries(1).unwrap().len(), 12);
    let after = backend.entry_detail(1, 0).unwrap();
    assert_eq!(after.path, before.path);
    assert_eq!(after.raw_idb_line, before.raw_idb_line);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn successful_reopen_queries_new_entry_table() {
    let root_a = temp_root("entries-reopen-a");
    write_entries_dist(&root_a);
    let root_b = temp_root("entries-reopen-b");
    write_idb(&root_b, "beta2", &[("beta2.sw.unix", "only")]);

    let mut backend = new_backend();
    backend.open_distribution(root_a.to_str().unwrap()).unwrap();
    assert_eq!(backend.entries(1).unwrap().len(), 12);

    backend.open_distribution(root_b.to_str().unwrap()).unwrap();
    let rows = backend.entries(1).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path, "only");
    assert_eq!(rows[0].subsystem, "beta2.sw.unix");
    // The new table is what the keys resolve against now.
    assert_eq!(backend.entry_detail(1, 0).unwrap().path, "only");
    assert_eq!(
        backend.entry_detail(1, 1).err().unwrap().to_string(),
        "entry 1 does not exist in product beta2"
    );

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

/// Entry smoke test against a real distribution directory, using the
/// same `SW_EXPLORER_TEST_DIST` opt-in as the other smoke tests.
#[test]
fn real_dist_entries_smoke() {
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
    let by_id: std::collections::HashMap<u64, &ffi::HierarchyNode> =
        nodes.iter().map(|node| (node.id, node)).collect();

    for node in &nodes {
        let rows = backend.entries(node.id).unwrap();
        // The scope count must agree with the hierarchy node exactly.
        assert_eq!(
            rows.len() as u64,
            node.entry_count,
            "entry count mismatch at scope {}",
            node.name
        );
        // Entry ids must be strictly increasing in every scope: the
        // original IDB order, never a regrouping.
        for pair in rows.windows(2) {
            assert!(
                pair[0].entry_id < pair[1].entry_id,
                "entry order broken at scope {}",
                node.name
            );
        }

        match node.kind {
            ffi::ObjectKind::Product => {
                // Product scope is the flat entry list itself.
                for (index, row) in rows.iter().enumerate() {
                    assert_eq!(row.entry_id, index as u64);
                    assert_eq!(row.product_id, node.id);
                }
                // Entry detail round-trips agree with the summary.
                if let Some(first) = rows.first() {
                    let detail = backend.entry_detail(node.id, first.entry_id).unwrap();
                    assert_eq!(detail.product_id, first.product_id);
                    assert_eq!(detail.entry_id, first.entry_id);
                    assert_eq!(detail.path, first.path);
                    assert_eq!(detail.subsystem, first.subsystem);
                    assert_eq!(detail.size_known, first.size_known);
                    assert_eq!(detail.size, first.size);
                    assert_eq!(detail.stored_size_known, first.stored_size_known);
                    assert_eq!(detail.stored_size, first.stored_size);
                    assert_eq!(detail.mach, first.mach);
                    assert_eq!(detail.unresolved_mach, first.unresolved_mach);
                }
            }
            ffi::ObjectKind::Image => {
                let prefix = format!("{}.", node.name);
                assert!(
                    rows.iter().all(|row| row.subsystem.starts_with(&prefix)),
                    "image scope {} leaked a foreign subsystem",
                    node.name
                );
            }
            ffi::ObjectKind::Subsystem => {
                let image = by_id[&node.parent_id].name.to_string();
                let identity = format!("{image}.{}", node.name);
                assert!(
                    rows.iter().all(|row| row.subsystem == identity),
                    "subsystem scope {identity} leaked a foreign subsystem"
                );
            }
            _ => panic!("unexpected object kind"),
        }
    }
}

/// Writes the standard search test distribution: two IDB-only
/// products, `alpha` and `beta`, whose records exercise global
/// scope, distribution ordering, duplicate paths, MACH variants and
/// unresolved MACH payloads.
///
/// `alpha` carries five records: `usr/bin/tool` in `alpha.sw.unix`,
/// three `usr/lib/libx.so` variants split over `alpha.sw.unix` and
/// `alpha.sw.extra` (two parsed MACH conditions, one unresolved),
/// and the path `libx` in `alpha.man.man`. `beta` carries a fourth
/// `usr/lib/libx.so` record and `usr/bin/other`, both in
/// `beta.sw.unix`.
///
/// Object ids: 1 = product `alpha`, 2 = image `alpha.sw`, 3 =
/// `unix`, 4 = `extra`, 5 = image `alpha.man`, 6 = `man`,
/// 7 = product `beta`, 8 = image `beta.sw`, 9 = `unix`.
fn write_search_dist(root: &Path) {
    write_raw_idb(
        root,
        "alpha",
        "f 0755 root sys usr/bin/tool src/tool alpha.sw.unix sum(1) size(10) cmpsize(0)\n\
         f 0644 root sys usr/lib/libx.so src/libx1 alpha.sw.unix sum(2) size(20) cmpsize(0) mach(CPUBOARD=IP22)\n\
         f 0644 root sys usr/lib/libx.so src/libx2 alpha.sw.extra sum(3) size(30) cmpsize(0) mach(CPUBOARD=IP26)\n\
         f 0644 root sys usr/lib/libx.so src/libx3 alpha.sw.extra sum(4) size(40) cmpsize(10) mach(=GARBAGE)\n\
         f 0644 root sys libx src/libx alpha.man.man sum(5) size(50) cmpsize(0)\n",
    );
    write_raw_idb(
        root,
        "beta",
        "f 0644 root sys usr/lib/libx.so src/libxb beta.sw.unix sum(6) size(60) cmpsize(0)\n\
         f 0644 root sys usr/bin/other src/other beta.sw.unix sum(7) size(70) cmpsize(0)\n",
    );
}

#[test]
fn search_requires_loaded_distribution() {
    let backend = new_backend();
    assert_eq!(
        backend.search_entries("libx").err().unwrap().to_string(),
        "no distribution loaded"
    );
}

#[test]
fn search_rejects_empty_query() {
    let root = temp_root("search-empty");
    write_search_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    assert_eq!(
        backend.search_entries("").err().unwrap().to_string(),
        "search query is empty"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_plain_text_is_a_substring_match() {
    let root = temp_root("search-substring");
    write_search_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // Plain text matches as *libx*: the three libx.so variants, the
    // path that is exactly "libx", and the beta record.
    let rows = backend.search_entries("libx").unwrap();
    let keys: Vec<(u64, u64)> = rows
        .iter()
        .map(|row| (row.product_id, row.entry_id))
        .collect();
    assert_eq!(keys, vec![(1, 1), (1, 2), (1, 3), (1, 4), (7, 0)]);

    let paths: Vec<&str> = rows.iter().map(|row| row.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "usr/lib/libx.so",
            "usr/lib/libx.so",
            "usr/lib/libx.so",
            "libx",
            "usr/lib/libx.so",
        ]
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_spans_products_images_and_subsystems() {
    let root = temp_root("search-global");
    write_search_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // The query hits every image and every subsystem of alpha, plus
    // beta: a search is never scoped to the tree selection.
    let rows = backend.search_entries("libx").unwrap();
    let subsystems: Vec<&str> = rows.iter().map(|row| row.subsystem.as_str()).collect();
    assert_eq!(
        subsystems,
        vec![
            "alpha.sw.unix",
            "alpha.sw.extra",
            "alpha.sw.extra",
            "alpha.man.man",
            "beta.sw.unix",
        ]
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_explicit_star_is_not_wrapped() {
    let root = temp_root("search-star");
    write_search_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // A pattern with its own wildcard reaches the matcher unchanged:
    // "libx*" requires the path to *start* with libx, so only the
    // bare "libx" record matches; a wrapped "*libx**" would match
    // every libx.so path as well.
    let rows = backend.search_entries("libx*").unwrap();
    let keys: Vec<(u64, u64)> = rows
        .iter()
        .map(|row| (row.product_id, row.entry_id))
        .collect();
    assert_eq!(keys, vec![(1, 4)]);
    assert_eq!(rows[0].path, "libx");

    // "*.so" matches only paths ending in .so: the bare "libx"
    // record, which a substring search would include, stays out.
    let rows = backend.search_entries("*.so").unwrap();
    let ids: Vec<u64> = rows.iter().map(|row| row.entry_id).collect();
    assert_eq!(ids, vec![1, 2, 3, 0]);
    assert!(rows.iter().all(|row| row.path.ends_with(".so")));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_explicit_question_mark_is_not_wrapped() {
    let root = temp_root("search-question");
    write_search_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // "?ibx" matches exactly one leading character: only the bare
    // "libx" path qualifies. A wrapped "*?ibx*" would also match
    // every "usr/lib/libx.so" record.
    let rows = backend.search_entries("?ibx").unwrap();
    let keys: Vec<(u64, u64)> = rows
        .iter()
        .map(|row| (row.product_id, row.entry_id))
        .collect();
    assert_eq!(keys, vec![(1, 4)]);
    assert_eq!(rows[0].path, "libx");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_keeps_product_and_idb_order() {
    let root = temp_root("search-order");
    write_search_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // Product order first: every alpha hit precedes the beta hit.
    // Within a product the original IDB order is exact, never an
    // alphabetical or subsystem-grouped rearrangement.
    let rows = backend.search_entries("libx").unwrap();
    for pair in rows.windows(2) {
        assert!(pair[0].product_id <= pair[1].product_id);
        if pair[0].product_id == pair[1].product_id {
            assert!(pair[0].entry_id < pair[1].entry_id);
        }
    }
    let ids: Vec<u64> = rows
        .iter()
        .filter(|row| row.product_id == 1)
        .map(|row| row.entry_id)
        .collect();
    assert_eq!(ids, vec![1, 2, 3, 4]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_never_deduplicates_duplicate_paths() {
    let root = temp_root("search-duplicates");
    write_search_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // Four IDB records share the path usr/lib/libx.so across the two
    // products: all four stay, each its own row.
    let rows = backend.search_entries("usr/lib/libx.so").unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|row| row.path == "usr/lib/libx.so"));
    let keys: Vec<(u64, u64)> = rows
        .iter()
        .map(|row| (row.product_id, row.entry_id))
        .collect();
    assert_eq!(keys, vec![(1, 1), (1, 2), (1, 3), (7, 0)]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_keeps_mach_variants_and_unresolved_payloads() {
    let root = temp_root("search-mach");
    write_search_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    let rows = backend.search_entries("usr/lib/libx.so").unwrap();
    // Every hardware-conditional variant survives the search, parsed
    // and unresolved payloads side by side.
    assert_eq!(rows[0].mach, vec!["CPUBOARD=IP22".to_string()]);
    assert!(rows[0].unresolved_mach.is_empty());
    assert_eq!(rows[1].mach, vec!["CPUBOARD=IP26".to_string()]);
    assert!(rows[1].unresolved_mach.is_empty());
    assert!(rows[2].mach.is_empty());
    assert_eq!(rows[2].unresolved_mach, vec!["=GARBAGE".to_string()]);
    assert!(rows[3].mach.is_empty());
    assert!(rows[3].unresolved_mach.is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_rows_share_summary_field_semantics() {
    let root = temp_root("search-fields");
    write_search_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // A search row is the same EntrySummary the scope listing
    // produces: same key, same decoded fields.
    let searched = backend.search_entries("usr/bin/tool").unwrap();
    assert_eq!(searched.len(), 1);
    let scoped = backend.entries(1).unwrap();
    let listed = &scoped[0];

    let row = &searched[0];
    assert_eq!(row.product_id, listed.product_id);
    assert_eq!(row.entry_id, listed.entry_id);
    assert_eq!(row.entry_id, 0);
    assert_eq!(row.path, listed.path);
    assert_eq!(row.subsystem, listed.subsystem);
    assert_eq!(row.file_type_raw, listed.file_type_raw);
    assert_eq!(row.size_known, listed.size_known);
    assert_eq!(row.size, listed.size);
    assert_eq!(row.stored_size_known, listed.stored_size_known);
    assert_eq!(row.stored_size, listed.stored_size);
    assert_eq!(row.mach, listed.mach);
    assert_eq!(row.unresolved_mach, listed.unresolved_mach);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_result_keys_resolve_through_entry_detail() {
    let root = temp_root("search-keys");
    write_search_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // Every search row carries a (product_id, entry_id) key the
    // inspector API resolves back to the same record.
    for row in backend.search_entries("libx").unwrap() {
        let detail = backend.entry_detail(row.product_id, row.entry_id).unwrap();
        assert_eq!(detail.product_id, row.product_id);
        assert_eq!(detail.entry_id, row.entry_id);
        assert_eq!(detail.path, row.path);
        assert_eq!(detail.subsystem, row.subsystem);
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_zero_results_is_not_an_error() {
    let root = temp_root("search-zero");
    write_search_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    let rows = backend.search_entries("definitely-not-a-file").unwrap();
    assert!(rows.is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn search_attributes_entries_to_their_owning_product() {
    let root = temp_root("search-ownership");
    // alpha's IDB carries a record naming a *foreign* subsystem:
    // sw-core keeps the record in alpha's entry list and derives
    // alpha's tree from the foreign name. The owner of entry id 0 is
    // alpha regardless of the beta.sw.unix text — and entry_detail
    // resolves keys against the owner's entry list.
    write_raw_idb(
        &root,
        "alpha",
        "f 0644 root sys usr/bin/foreign src/foreign beta.sw.unix sum(1) size(1) cmpsize(0)\n",
    );
    write_raw_idb(
        &root,
        "beta",
        "f 0644 root sys usr/bin/actual-beta src/actual beta.sw.unix sum(2) size(2) cmpsize(0)\n",
    );

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    let nodes = backend.hierarchy().unwrap();
    let product_id = |name: &str| {
        nodes
            .iter()
            .find(|node| matches!(node.kind, ffi::ObjectKind::Product) && node.name == name)
            .unwrap()
            .id
    };
    let alpha_id = product_id("alpha");
    let beta_id = product_id("beta");
    assert_ne!(alpha_id, beta_id);

    // The hit must carry the owning product's key: resolving it
    // through the inspector API reads alpha's first record, never
    // beta's.
    let rows = backend.search_entries("foreign").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].product_id, alpha_id);
    assert_eq!(rows[0].entry_id, 0);
    // The record's content is untouched: it still names beta.sw.unix.
    assert_eq!(rows[0].subsystem, "beta.sw.unix");

    let detail = backend
        .entry_detail(rows[0].product_id, rows[0].entry_id)
        .unwrap();
    assert_eq!(detail.path, "usr/bin/foreign");
    assert_eq!(detail.subsystem, "beta.sw.unix");

    // Beta's own record keeps resolving to beta.
    let rows = backend.search_entries("actual-beta").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].product_id, beta_id);
    assert_eq!(rows[0].entry_id, 0);
    let detail = backend
        .entry_detail(rows[0].product_id, rows[0].entry_id)
        .unwrap();
    assert_eq!(detail.path, "usr/bin/actual-beta");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn failed_open_keeps_previous_search_results() {
    let root = temp_root("search-keep");
    write_search_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    let before = backend.search_entries("libx").unwrap();

    let missing = root.join("missing");
    assert!(
        backend
            .open_distribution(missing.to_str().unwrap())
            .is_err()
    );

    // The failed open touched nothing: the same search still answers
    // from the previously loaded distribution.
    let after = backend.search_entries("libx").unwrap();
    let before_keys: Vec<(u64, u64)> = before
        .iter()
        .map(|row| (row.product_id, row.entry_id))
        .collect();
    let after_keys: Vec<(u64, u64)> = after
        .iter()
        .map(|row| (row.product_id, row.entry_id))
        .collect();
    assert_eq!(before_keys, after_keys);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn successful_reopen_searches_the_new_distribution() {
    let root_a = temp_root("search-reopen-a");
    write_search_dist(&root_a);
    let root_b = temp_root("search-reopen-b");
    write_idb(&root_b, "beta2", &[("beta2.sw.unix", "only")]);

    let mut backend = new_backend();
    backend.open_distribution(root_a.to_str().unwrap()).unwrap();
    assert_eq!(backend.search_entries("libx").unwrap().len(), 5);

    backend.open_distribution(root_b.to_str().unwrap()).unwrap();
    assert!(backend.search_entries("libx").unwrap().is_empty());
    let rows = backend.search_entries("only").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].product_id, 1);
    assert_eq!(rows[0].entry_id, 0);
    assert_eq!(rows[0].subsystem, "beta2.sw.unix");

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

/// Search smoke test against a real distribution directory, using the
/// same `SW_EXPLORER_TEST_DIST` opt-in as the other smoke tests.
///
/// Every query is cross-checked against the core query the CLI runs
/// for the same text, so the bridge can never drift from `sw find`.
#[test]
fn real_dist_search_smoke() {
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
    let distribution = sw_core::distribution::Distribution::open(&path).unwrap();

    for text in ["Xsgi", "libGL"] {
        let hits = backend.search_entries(text).unwrap();
        assert!(!hits.is_empty(), "expected {text} hits in a full media set");
        // Plain text is a substring search: every hit contains the text.
        assert!(hits.iter().all(|row| row.path.contains(text)));

        // Distribution product order, per-product IDB order: product
        // ids never decrease, entry ids strictly increase within one.
        for pair in hits.windows(2) {
            assert!(pair[0].product_id <= pair[1].product_id);
            if pair[0].product_id == pair[1].product_id {
                assert!(pair[0].entry_id < pair[1].entry_id);
            }
        }

        // The bridge must return exactly what the core query
        // returns — the same semantics the CLI exposes; the explicit
        // wildcard spelling of the same pattern agrees as well.
        let expected = distribution
            .find(&sw_core::query::Query::path(format!("*{text}*")))
            .entries
            .len();
        assert_eq!(hits.len(), expected, "substring count mismatch for {text}");
        let wildcard = backend.search_entries(&format!("*{text}*")).unwrap();
        assert_eq!(
            wildcard.len(),
            expected,
            "wildcard count mismatch for {text}"
        );
    }

    // An explicit path wildcard behaves like `sw find usr/lib/*.so`.
    let so_hits = backend.search_entries("usr/lib/*.so").unwrap();
    assert!(!so_hits.is_empty());
    assert!(
        so_hits
            .iter()
            .all(|row| row.path.starts_with("usr/lib/") && row.path.ends_with(".so"))
    );
    let expected_so = distribution
        .find(&sw_core::query::Query::path("usr/lib/*.so"))
        .entries
        .len();
    assert_eq!(so_hits.len(), expected_so);

    // Every hit resolves through the inspector API.
    let first = backend.search_entries("Xsgi").unwrap();
    let row = first.first().unwrap();
    let detail = backend.entry_detail(row.product_id, row.entry_id).unwrap();
    assert_eq!(detail.path, row.path);
    assert_eq!(detail.subsystem, row.subsystem);
}
