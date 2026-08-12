//! Tests for the bridge: object-id behaviour, hierarchy flattening,
//! detail DTO semantics, and opt-in smoke tests against a real
//! distribution.
use crate::backend::new_backend;
use crate::bridge::ffi;
use std::path::{Path, PathBuf};
use sw_core::mach::eval::HardwareProfile;

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

/// Writes a malformed-but-preserved descriptor for `product`: two
/// image records both named `sw`, each carrying one `unix` subsystem
/// record, so one product holds two logical images (and subsystems)
/// with identical qualified names.
fn write_duplicate_image_descriptor(root: &Path, product: &str) {
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
    bytes.extend_from_slice(&2u16.to_be_bytes()); // image count

    for _ in 0..2 {
        bytes.extend_from_slice(&0x0858u16.to_be_bytes()); // image flags
        lp16(&mut bytes, "sw");
        lp16(&mut bytes, "System Software");
        bytes.extend_from_slice(&0u16.to_be_bytes()); // legacy field
        bytes.extend_from_slice(&9999u16.to_be_bytes()); // order
        bytes.extend_from_slice(&1u32.to_be_bytes()); // version
        bytes.extend_from_slice(&0u32.to_be_bytes()); // reserved
        bytes.extend_from_slice(&0u16.to_be_bytes()); // metadata count
        bytes.extend_from_slice(&1u16.to_be_bytes()); // subsystem count

        bytes.extend_from_slice(&0x0852u16.to_be_bytes()); // flags
        lp16(&mut bytes, "unix");
        lp16(&mut bytes, "Subsystem Title");
        lp16(&mut bytes, "ALL");
        for _ in 0..9 {
            bytes.extend_from_slice(&0u16.to_be_bytes()); // rule slots
        }
    }
    std::fs::write(root.join(product), bytes).unwrap();
}
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
/// points), image attributes (mach), image version/order and full
/// subsystem records with rules and attributes.
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
    image_attributes: Vec<(u8, Vec<u8>)>,
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
    push_attributes(&mut bytes, &image_attributes);
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
        vec![],
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
    // The summary counts every diagnostic, at both storage levels:
    // one distribution-level error (the invalid product name) plus the
    // two product-level warnings of `test` (no descriptor file, missing
    // image archive).
    assert_eq!(summary.diagnostic_count, 3);
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
fn hierarchy_keeps_same_name_images_as_distinct_objects() {
    let root = temp_root("same-name-hierarchy");
    write_foreign_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    let nodes = backend.hierarchy().unwrap();

    // alpha grows a synthetic beta.sw from its foreign IDB reference;
    // beta carries its own beta.sw. Two logical images, one media
    // name: distinct object ids, distinct parents, the same display
    // name — the honest representation of the media facts.
    let shapes: Vec<_> = nodes.iter().map(node_shape).collect();
    assert_eq!(
        shapes,
        vec![
            (1, 0, "product", "alpha".to_string(), 1, false, true),
            (2, 1, "image", "beta.sw".to_string(), 1, false, true),
            (3, 2, "subsystem", "unix".to_string(), 1, false, true),
            (4, 0, "product", "beta".to_string(), 1, false, true),
            (5, 4, "image", "beta.sw".to_string(), 1, false, true),
            (6, 5, "subsystem", "unix".to_string(), 1, false, true),
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
fn detail_reports_the_containing_product_not_the_name_segment() {
    let root = temp_root("foreign-detail");
    write_foreign_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // The synthetic beta.sw under alpha (object 2): the displayed
    // name is the qualified media name, the containing product is the
    // actual owner — the two legitimately disagree.
    let image = backend.image_detail(2).unwrap();
    assert_eq!(image.name, "beta.sw");
    assert_eq!(image.product_name, "alpha");
    let subsystem = backend.subsystem_detail(3).unwrap();
    assert_eq!(subsystem.identity, "beta.sw.unix");
    assert_eq!(subsystem.product_name, "alpha");
    assert_eq!(subsystem.image_name, "beta.sw");

    // beta's own beta.sw (object 5): name segment and container agree.
    let image = backend.image_detail(5).unwrap();
    assert_eq!(image.name, "beta.sw");
    assert_eq!(image.product_name, "beta");
    let subsystem = backend.subsystem_detail(6).unwrap();
    assert_eq!(subsystem.identity, "beta.sw.unix");
    assert_eq!(subsystem.product_name, "beta");
    assert_eq!(subsystem.image_name, "beta.sw");

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
fn scopes_of_same_name_objects_use_exact_attachment() {
    let root = temp_root("foreign-scopes");
    write_foreign_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // Object 2 is alpha's synthetic beta.sw, object 5 beta's own: the
    // scope of each is the attachment of that exact object, never a
    // cross-product name filter.
    let alpha_rows = backend.entries(2).unwrap();
    assert_eq!(alpha_rows.len(), 1);
    assert_eq!(alpha_rows[0].path, "usr/bin/foreign");
    assert_eq!(alpha_rows[0].product_id, 1);
    let beta_rows = backend.entries(5).unwrap();
    assert_eq!(beta_rows.len(), 1);
    assert_eq!(beta_rows[0].path, "usr/bin/actual-beta");
    assert_eq!(beta_rows[0].product_id, 4);

    // Subsystem scopes (3 = alpha's unix, 6 = beta's unix) likewise.
    let alpha_rows = backend.entries(3).unwrap();
    assert_eq!(alpha_rows.len(), 1);
    assert_eq!(alpha_rows[0].path, "usr/bin/foreign");
    let beta_rows = backend.entries(6).unwrap();
    assert_eq!(beta_rows.len(), 1);
    assert_eq!(beta_rows[0].path, "usr/bin/actual-beta");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn same_owner_duplicate_images_have_disjoint_scopes() {
    let root = temp_root("dup-image-scopes");
    write_duplicate_image_descriptor(&root, "dup");
    write_raw_idb(
        &root,
        "dup",
        "f 0644 root sys a.txt src/a dup.sw.unix sum(1) size(1) cmpsize(0)\n\
         f 0644 root sys b.txt src/b dup.sw.unix sum(2) size(1) cmpsize(0)\n",
    );

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // Object ids: 1 = dup, 2 = dup.sw #0, 3 = unix #0, 4 = dup.sw #1,
    // 5 = unix #1. Attachment assigns both records to the first
    // matching subsystem; the second image's scope stays empty — a
    // name-based scope would list the records under both images.
    let shapes: Vec<_> = backend
        .hierarchy()
        .unwrap()
        .iter()
        .map(node_shape)
        .collect();
    assert_eq!(
        shapes,
        vec![
            (1, 0, "product", "dup".to_string(), 2, true, true),
            (2, 1, "image", "dup.sw".to_string(), 2, true, true),
            (3, 2, "subsystem", "unix".to_string(), 2, true, true),
            (4, 1, "image", "dup.sw".to_string(), 0, true, false),
            (5, 4, "subsystem", "unix".to_string(), 0, true, false),
        ]
    );

    let first = backend.entries(2).unwrap();
    let paths: Vec<&str> = first.iter().map(|row| row.path.as_str()).collect();
    assert_eq!(paths, vec!["a.txt", "b.txt"]);
    assert!(backend.entries(4).unwrap().is_empty());
    assert_eq!(backend.entries(3).unwrap().len(), 2);
    assert!(backend.entries(5).unwrap().is_empty());

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

// ---------------------------------------------------------------------------
// Hardware candidates and hardware-profile selection
// ---------------------------------------------------------------------------

/// One hardware attribute/value pair, the bridge input shape.
fn hw(attribute: &str, value: &str) -> ffi::HardwareValue {
    ffi::HardwareValue {
        attribute: attribute.to_string(),
        value: value.to_string(),
    }
}

/// The (product id, entry id) pairs of a selection key list.
fn key_pairs(keys: &[ffi::SelectionEntryKey]) -> Vec<(u64, u64)> {
    keys.iter()
        .map(|key| (key.product_id, key.entry_id))
        .collect()
}

/// The (attribute, values) pairs of a candidate list.
fn candidate_pairs(candidates: &[ffi::HardwareCandidateSet]) -> Vec<(String, Vec<String>)> {
    candidates
        .iter()
        .map(|set| (set.attribute.clone(), set.values.clone()))
        .collect()
}

/// Object id of the product called `name`, looked up in the hierarchy
/// — never derived from id arithmetic.
fn product_object_id(backend: &crate::backend::Backend, name: &str) -> u64 {
    backend
        .hierarchy()
        .unwrap()
        .iter()
        .find(|node| matches!(node.kind, ffi::ObjectKind::Product) && node.name == name)
        .map(|node| node.id)
        .unwrap_or_else(|| panic!("product {name} not found"))
}

/// Writes the standard selection test distribution: one descriptor+
/// IDB product `sel` whose entries exercise mach-specific selection,
/// mach-less fallback, duplicate-path conflicts and unparsed MACH,
/// plus one descriptor subsystem (`broken`) whose MACH payload cannot
/// be parsed.
///
/// Object ids: 1 = product `sel`, 2 = image `sel.sw`, 3 = `unix`,
/// 4 = `broken`. Entry ids 0–10 in IDB order.
fn write_selection_dist(root: &Path) {
    let unix = SubsystemSpec::plain("unix", "UNIX", "ALL", 0x0850);
    let broken = SubsystemSpec {
        attributes: vec![(b'm', b"=BROKEN".to_vec())],
        ..SubsystemSpec::plain("broken", "Broken", "ALL", 0x0850)
    };
    write_rich_descriptor(
        root,
        "sel",
        "Selection Product",
        1,
        vec![],
        "sw",
        "System Software",
        1,
        1,
        vec![],
        &[unix, broken],
    );
    write_raw_idb(
        root,
        "sel",
        "f 0755 root sys usr/bin/plain src/plain sel.sw.unix sum(1) size(10) cmpsize(0)\n\
         f 0755 root sys usr/bin/board src/board sel.sw.unix sum(1) size(10) cmpsize(0) mach(CPUBOARD=IP22)\n\
         f 0755 root sys usr/bin/board26 src/board26 sel.sw.unix sum(1) size(10) cmpsize(0) mach(IP26)\n\
         f 0644 root sys usr/lib/dup src/dup1 sel.sw.unix sum(1) size(10) cmpsize(0) mach(CPUBOARD=IP22)\n\
         f 0644 root sys usr/lib/dup src/dup2 sel.sw.unix sum(1) size(10) cmpsize(0)\n\
         f 0644 root sys usr/lib/conf src/conf4 sel.sw.unix sum(1) size(10) cmpsize(0) mach(CPUARCH=R4000)\n\
         f 0644 root sys usr/lib/conf src/conf5 sel.sw.unix sum(1) size(10) cmpsize(0) mach(CPUARCH=R5000)\n\
         f 0644 root sys usr/lib/fb src/fb1 sel.sw.unix sum(1) size(10) cmpsize(0)\n\
         f 0644 root sys usr/lib/fb src/fb2 sel.sw.unix sum(1) size(10) cmpsize(0)\n\
         f 0644 root sys usr/lib/broken src/broken sel.sw.unix sum(1) size(10) cmpsize(0) mach(=GARBAGE)\n\
         f 0755 root sys usr/bin/kern src/kern sel.sw.broken sum(1) size(10) cmpsize(0)\n",
    );
}

/// Writes the empty-value selection distribution: one product `gfx`
/// whose IDB carries a mach-less record and one record restricted to
/// headless boards through `mach(GFXBOARD=)` — the empty right-hand
/// side real media use.
///
/// Object ids: 1 = product `gfx`, 2 = image `gfx.sw`, 3 = `unix`.
/// Entry ids 0–1 in IDB order.
fn write_empty_value_dist(root: &Path) {
    write_rich_descriptor(
        root,
        "gfx",
        "Gfx Product",
        1,
        vec![],
        "sw",
        "System Software",
        1,
        1,
        vec![],
        &[SubsystemSpec::plain("unix", "UNIX", "ALL", 0x0850)],
    );
    write_raw_idb(
        root,
        "gfx",
        "f 0755 root sys usr/bin/any src/any gfx.sw.unix sum(1) size(10) cmpsize(0)\n\
         f 0755 root sys usr/bin/headless src/headless gfx.sw.unix sum(1) size(10) cmpsize(0) mach(GFXBOARD=)\n",
    );
}

/// Writes the hierarchy-level selection distribution: `imach` carries
/// an image MACH restriction, `pmach` a product restriction and
/// `smach` a subsystem restriction; each product holds one mach-less
/// entry.
fn write_level_mach_dist(root: &Path) {
    write_rich_descriptor(
        root,
        "imach",
        "Image Mach Product",
        1,
        vec![],
        "sw",
        "System Software",
        1,
        1,
        vec![(b'm', b"GFXBOARD=EXPRESS".to_vec())],
        &[SubsystemSpec::plain("unix", "UNIX", "ALL", 0x0850)],
    );
    write_idb(root, "imach", &[("imach.sw.unix", "usr/bin/imach")]);

    write_rich_descriptor(
        root,
        "pmach",
        "Product Mach Product",
        1,
        vec![(b'm', b"CPUBOARD=IP22".to_vec())],
        "sw",
        "System Software",
        1,
        1,
        vec![],
        &[SubsystemSpec::plain("unix", "UNIX", "ALL", 0x0850)],
    );
    write_idb(root, "pmach", &[("pmach.sw.unix", "usr/bin/pmach")]);

    let unix = SubsystemSpec {
        attributes: vec![(b'm', b"MODE=64bit".to_vec())],
        ..SubsystemSpec::plain("unix", "UNIX", "ALL", 0x0850)
    };
    write_rich_descriptor(
        root,
        "smach",
        "Subsystem Mach Product",
        1,
        vec![],
        "sw",
        "System Software",
        1,
        1,
        vec![],
        &[unix],
    );
    write_idb(root, "smach", &[("smach.sw.unix", "usr/bin/smach")]);
}

/// Writes the selection ownership regression distribution: `alpha`'s
/// IDB carries a record naming the *foreign* subsystem `beta.sw.unix`.
/// The entry still belongs to alpha, and selection keys must say so.
///
/// Object ids: 1 = product `alpha`, 2 = image `beta.sw`, 3 = `unix`,
/// 4 = product `beta`, 5 = image `beta.sw`, 6 = `unix`.
fn write_foreign_selection_dist(root: &Path) {
    write_raw_idb(
        root,
        "alpha",
        "f 0644 root sys usr/bin/foreign src/foreign beta.sw.unix sum(1) size(1) cmpsize(0)\n",
    );
    write_raw_idb(
        root,
        "beta",
        "f 0644 root sys usr/bin/actual-beta src/b beta.sw.unix sum(1) size(1) cmpsize(0)\n",
    );
}

/// Writes the candidate-extraction test distribution: one product
/// `cand` carrying MACH expressions on every hierarchy level and on
/// its entries, covering nested `&&`/`||`/`!`, several comparison
/// operators, bare-value shorthand, an unknown attribute, repeated
/// values and an unparseable payload. The subsystem also carries a
/// conditional-flag condition (`IP99`) that is *not* a MACH
/// expression and must never surface as a candidate.
///
/// Object ids: 1 = product `cand`, 2 = image `cand.sw`, 3 = `unix`.
fn write_candidates_dist(root: &Path) {
    let unix = SubsystemSpec {
        attributes: vec![
            (b'm', b"MODE>=64bit".to_vec()),
            (b'm', b"CPUBOARD<IP30".to_vec()),
            (b'D', b"CPUBOARD=IP99".to_vec()),
        ],
        ..SubsystemSpec::plain("unix", "UNIX", "ALL", 0x0850)
    };
    write_rich_descriptor(
        root,
        "cand",
        "Candidate Product",
        1,
        vec![(
            b'm',
            b"CPUBOARD=IP22 && (GFXBOARD=EXPRESS || GFXBOARD=NEWPRESS)".to_vec(),
        )],
        "sw",
        "System Software",
        1,
        1,
        vec![
            (b'm', b"GFXBOARD!=SERVER".to_vec()),
            (b'm', b"!(VIDEO=EVO)".to_vec()),
        ],
        &[unix],
    );
    write_raw_idb(
        root,
        "cand",
        "f 0755 root sys usr/bin/board26 src/b26 cand.sw.unix sum(1) size(10) cmpsize(0) mach(IP26)\n\
         f 0755 root sys usr/bin/frob src/frob cand.sw.unix sum(1) size(10) cmpsize(0) mach(FROBNICATE=YES)\n\
         f 0755 root sys usr/bin/legacy src/legacy cand.sw.unix sum(1) size(10) cmpsize(0) mach(CPUBOARD=IP22 GFXBOARD=EXPRESS)\n\
         f 0755 root sys usr/bin/garbage src/garbage cand.sw.unix sum(1) size(10) cmpsize(0) mach(=GARBAGE)\n\
         f 0755 root sys usr/bin/again src/again cand.sw.unix sum(1) size(10) cmpsize(0) mach(CPUBOARD=IP22)\n",
    );
}

#[test]
fn candidates_require_loaded_distribution() {
    let backend = new_backend();
    assert_eq!(
        backend.hardware_candidates().err().unwrap().to_string(),
        "no distribution loaded"
    );
}

#[test]
fn candidates_match_core_discovery() {
    let root = temp_root("candidates-core-conversion");
    write_candidates_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // The bridge is a pure conversion: its DTOs equal the core
    // candidate sets field by field, in the same order. The candidate
    // semantics themselves are golden-tested in sw-core.
    let distribution = sw_core::distribution::Distribution::open(&root).unwrap();
    let core = distribution.hardware_candidates();
    let bridge = backend.hardware_candidates().unwrap();
    assert_eq!(bridge.len(), core.len());
    assert!(!bridge.is_empty());
    for (ffi_set, core_set) in bridge.iter().zip(core.iter()) {
        assert_eq!(ffi_set.attribute, core_set.attribute);
        assert_eq!(ffi_set.values, core_set.values);
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn candidates_survive_failed_open() {
    let root = temp_root("candidates-failed-open");
    write_candidates_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    let before = backend.hardware_candidates().unwrap();

    backend
        .open_distribution("/definitely/missing/distribution")
        .err()
        .unwrap();
    assert_eq!(
        candidate_pairs(&before),
        candidate_pairs(&backend.hardware_candidates().unwrap())
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn candidates_follow_successful_reopen() {
    let root_a = temp_root("candidates-reopen-a");
    write_candidates_dist(&root_a);
    let root_b = temp_root("candidates-reopen-b");
    write_selection_dist(&root_b);

    let mut backend = new_backend();
    backend.open_distribution(root_a.to_str().unwrap()).unwrap();
    assert!(
        backend
            .hardware_candidates()
            .unwrap()
            .iter()
            .any(|set| set.attribute == "FROBNICATE")
    );

    backend.open_distribution(root_b.to_str().unwrap()).unwrap();
    let after = candidate_pairs(&backend.hardware_candidates().unwrap());
    assert_eq!(
        after,
        vec![
            (
                "CPUBOARD".to_string(),
                vec!["IP22".to_string(), "IP26".to_string()]
            ),
            (
                "CPUARCH".to_string(),
                vec!["R4000".to_string(), "R5000".to_string()],
            ),
        ]
    );

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

#[test]
fn selection_requires_loaded_distribution() {
    let backend = new_backend();
    assert_eq!(
        backend
            .select_entries(vec![hw("CPUBOARD", "IP22")])
            .err()
            .unwrap()
            .to_string(),
        "no distribution loaded"
    );
}

#[test]
fn selection_rejects_empty_attribute_name() {
    let root = temp_root("selection-blank");
    write_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    assert_eq!(
        backend
            .select_entries(vec![hw("", "IP22")])
            .err()
            .unwrap()
            .to_string(),
        "hardware attribute name is empty"
    );

    // Unknown attribute and value names are not rejected.
    backend
        .select_entries(vec![hw("FROBNICATE", "YES")])
        .unwrap();

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_empty_value_matches_core_select() {
    let root = temp_root("selection-empty-value");
    write_empty_value_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    let distribution = sw_core::distribution::Distribution::open(&root).unwrap();

    // The empty right-hand side is parsed from the media and offered
    // as a candidate like any other value.
    let candidates = backend.hardware_candidates().unwrap();
    assert_eq!(
        candidate_pairs(&candidates),
        vec![("GFXBOARD".to_string(), vec![String::new()])]
    );

    // The empty value is a genuine hardware fact: it flows into the
    // profile unchanged and the bridge selection matches the core
    // selection entry for entry.
    let pairs = vec![hw("GFXBOARD", "")];
    let mut builder = HardwareProfile::builder();
    for pair in &pairs {
        builder = builder.add(&pair.attribute, &pair.value);
    }
    let core = distribution.select(&builder.build());
    let snapshot = backend.select_entries(pairs).unwrap();

    assert_eq!(snapshot.selected.len(), core.selected.len());
    assert_eq!(snapshot.conflicts.len(), core.conflicts.len());
    let bridge_paths: Vec<String> = snapshot
        .selected
        .iter()
        .map(|key| {
            backend
                .entry_detail(key.product_id, key.entry_id)
                .unwrap()
                .path
        })
        .collect();
    let core_paths: Vec<String> = core
        .selected
        .iter()
        .map(|located| located.entry.path.to_string())
        .collect();
    assert_eq!(bridge_paths, core_paths);

    // The empty value is not inert: it selects the headless record a
    // non-empty value rejects, while the mach-less record is selected
    // either way.
    assert!(bridge_paths.iter().any(|path| path == "usr/bin/headless"));
    let express_paths: Vec<String> = backend
        .select_entries(vec![hw("GFXBOARD", "EXPRESS")])
        .unwrap()
        .selected
        .iter()
        .map(|key| {
            backend
                .entry_detail(key.product_id, key.entry_id)
                .unwrap()
                .path
        })
        .collect();
    assert!(!express_paths.iter().any(|path| path == "usr/bin/headless"));
    assert!(express_paths.iter().any(|path| path == "usr/bin/any"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_selects_ordinary_entries() {
    let root = temp_root("selection-ordinary");
    write_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // CPUBOARD=IP22: the plain fallback, the IP22 entry and the
    // IP22-specific duplicate win; the two fallbacks of usr/lib/fb
    // conflict; the unparsed entry and the unresolved subsystem's
    // entry are conflicts but never selected. Entry id 0 is a
    // perfectly valid selected id.
    let snapshot = backend
        .select_entries(vec![hw("CPUBOARD", "IP22")])
        .unwrap();
    assert_eq!(
        key_pairs(&snapshot.selected),
        vec![(1, 0), (1, 1), (1, 3), (1, 7), (1, 8)]
    );
    let conflicts: Vec<(String, Vec<(u64, u64)>)> = snapshot
        .conflicts
        .iter()
        .map(|conflict| (conflict.path.clone(), key_pairs(&conflict.candidates)))
        .collect();
    assert_eq!(
        conflicts,
        vec![
            ("usr/lib/fb".to_string(), vec![(1, 7), (1, 8)]),
            ("usr/lib/broken".to_string(), vec![(1, 9)]),
            ("usr/bin/kern".to_string(), vec![(1, 10)]),
        ]
    );

    // Every selected key resolves back to its entry.
    for key in &snapshot.selected {
        backend.entry_detail(key.product_id, key.entry_id).unwrap();
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_specific_beats_fallback() {
    let root = temp_root("selection-specific");
    write_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // The IP22-specific duplicate wins over the mach-less fallback.
    let snapshot = backend
        .select_entries(vec![hw("CPUBOARD", "IP22")])
        .unwrap();
    let selected = key_pairs(&snapshot.selected);
    assert!(selected.contains(&(1, 3)));
    assert!(!selected.contains(&(1, 4)));

    // With a non-matching board the fallback is selected instead.
    let snapshot = backend
        .select_entries(vec![hw("CPUBOARD", "IP19")])
        .unwrap();
    let selected = key_pairs(&snapshot.selected);
    assert!(!selected.contains(&(1, 3)));
    assert!(selected.contains(&(1, 4)));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_multiple_matching_specific_entries_conflict() {
    let root = temp_root("selection-specific-conflict");
    write_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // Both CPUARCH-specific records match: both stay selected and the
    // path is reported as a conflict with both candidates.
    let snapshot = backend
        .select_entries(vec![hw("CPUARCH", "R4000"), hw("CPUARCH", "R5000")])
        .unwrap();
    let selected = key_pairs(&snapshot.selected);
    assert!(selected.contains(&(1, 5)));
    assert!(selected.contains(&(1, 6)));
    let conflict = snapshot
        .conflicts
        .iter()
        .find(|conflict| conflict.path == "usr/lib/conf")
        .unwrap();
    assert_eq!(key_pairs(&conflict.candidates), vec![(1, 5), (1, 6)]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_multiple_fallbacks_conflict() {
    let root = temp_root("selection-fallback-conflict");
    write_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // Two mach-less records share usr/lib/fb: both are selected and
    // the path conflicts, under any profile.
    let snapshot = backend
        .select_entries(vec![hw("CPUBOARD", "IP19")])
        .unwrap();
    let selected = key_pairs(&snapshot.selected);
    assert!(selected.contains(&(1, 7)));
    assert!(selected.contains(&(1, 8)));
    let conflict = snapshot
        .conflicts
        .iter()
        .find(|conflict| conflict.path == "usr/lib/fb")
        .unwrap();
    assert_eq!(key_pairs(&conflict.candidates), vec![(1, 7), (1, 8)]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_unparsed_entry_mach_conflicts_but_is_never_selected() {
    let root = temp_root("selection-unparsed-entry");
    write_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // mach(=GARBAGE) cannot be evaluated: the record is reported as a
    // conflict candidate and stays out of the selected set.
    let snapshot = backend
        .select_entries(vec![hw("CPUBOARD", "IP22")])
        .unwrap();
    assert!(!key_pairs(&snapshot.selected).contains(&(1, 9)));
    let conflict = snapshot
        .conflicts
        .iter()
        .find(|conflict| conflict.path == "usr/lib/broken")
        .unwrap();
    assert_eq!(key_pairs(&conflict.candidates), vec![(1, 9)]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_unresolved_hierarchy_mach_conflicts_but_is_never_selected() {
    let root = temp_root("selection-unresolved-subsystem");
    write_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // The `broken` subsystem's descriptor MACH payload does not
    // parse: its entry's applicability is unknown, so the path is a
    // conflict and the record is not selected.
    let snapshot = backend
        .select_entries(vec![hw("CPUBOARD", "IP22")])
        .unwrap();
    assert!(!key_pairs(&snapshot.selected).contains(&(1, 10)));
    let conflict = snapshot
        .conflicts
        .iter()
        .find(|conflict| conflict.path == "usr/bin/kern")
        .unwrap();
    assert_eq!(key_pairs(&conflict.candidates), vec![(1, 10)]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_respects_every_hierarchy_level() {
    let root = temp_root("selection-levels");
    write_level_mach_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    let imach = product_object_id(&backend, "imach");
    let pmach = product_object_id(&backend, "pmach");
    let smach = product_object_id(&backend, "smach");

    // All three restrictions match: every product's entry is
    // selected.
    let snapshot = backend
        .select_entries(vec![
            hw("CPUBOARD", "IP22"),
            hw("GFXBOARD", "EXPRESS"),
            hw("MODE", "64bit"),
        ])
        .unwrap();
    assert_eq!(
        key_pairs(&snapshot.selected),
        vec![(imach, 0), (pmach, 0), (smach, 0)]
    );

    // Only the product-level restriction matches: the image and
    // subsystem restrictions exclude their products.
    let snapshot = backend
        .select_entries(vec![hw("CPUBOARD", "IP22")])
        .unwrap();
    assert_eq!(key_pairs(&snapshot.selected), vec![(pmach, 0)]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_multi_valued_attributes() {
    let root = temp_root("selection-multi-value");
    write_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // One attribute carrying several values: CPUARCH=MIPS2 alone
    // matches nothing, but the R4000 alternative does.
    let snapshot = backend
        .select_entries(vec![hw("CPUARCH", "MIPS2"), hw("CPUARCH", "R4000")])
        .unwrap();
    let selected = key_pairs(&snapshot.selected);
    assert!(selected.contains(&(1, 5)));
    assert!(!selected.contains(&(1, 6)));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_keys_identify_the_owning_product() {
    let root = temp_root("selection-ownership");
    write_foreign_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    let alpha = product_object_id(&backend, "alpha");
    let beta = product_object_id(&backend, "beta");

    // Both records are mach-less fallbacks and therefore selected.
    // The record naming the foreign beta.sw.unix subsystem belongs to
    // alpha: its key carries alpha's product object id, and the
    // inspector resolves it back to alpha's record.
    let snapshot = backend
        .select_entries(vec![hw("CPUBOARD", "IP22")])
        .unwrap();
    assert_eq!(key_pairs(&snapshot.selected), vec![(alpha, 0), (beta, 0)]);

    let foreign = backend.entry_detail(alpha, 0).unwrap();
    assert_eq!(foreign.path, "usr/bin/foreign");
    assert_eq!(foreign.subsystem, "beta.sw.unix");
    let actual_beta = backend.entry_detail(beta, 0).unwrap();
    assert_eq!(actual_beta.path, "usr/bin/actual-beta");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_empty_profile_does_not_panic() {
    let root = temp_root("selection-empty-profile");
    write_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();

    // An empty profile matches no comparison: mach-less fallbacks are
    // selected, mach-specific records are not, unknown applicability
    // still conflicts.
    let snapshot = backend.select_entries(vec![]).unwrap();
    let selected = key_pairs(&snapshot.selected);
    assert!(selected.contains(&(1, 0)));
    assert!(selected.contains(&(1, 4)));
    assert!(!selected.contains(&(1, 1)));
    assert!(!selected.contains(&(1, 3)));
    assert_eq!(snapshot.conflicts.len(), 3);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_survives_failed_open() {
    let root = temp_root("selection-failed-open");
    write_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    let before = backend
        .select_entries(vec![hw("CPUBOARD", "IP22")])
        .unwrap();

    backend
        .open_distribution("/definitely/missing/distribution")
        .err()
        .unwrap();
    let after = backend
        .select_entries(vec![hw("CPUBOARD", "IP22")])
        .unwrap();
    assert_eq!(key_pairs(&before.selected), key_pairs(&after.selected));
    assert_eq!(before.conflicts.len(), after.conflicts.len());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selection_follows_successful_reopen() {
    let root_a = temp_root("selection-reopen-a");
    write_selection_dist(&root_a);
    let root_b = temp_root("selection-reopen-b");
    write_idb(&root_b, "gamma", &[("gamma.sw.unix", "usr/bin/gamma")]);

    let mut backend = new_backend();
    backend.open_distribution(root_a.to_str().unwrap()).unwrap();
    let before = backend
        .select_entries(vec![hw("CPUBOARD", "IP22")])
        .unwrap();
    assert!(before.selected.len() > 1);

    backend.open_distribution(root_b.to_str().unwrap()).unwrap();
    let after = backend
        .select_entries(vec![hw("CPUBOARD", "IP22")])
        .unwrap();
    let gamma = product_object_id(&backend, "gamma");
    assert_eq!(key_pairs(&after.selected), vec![(gamma, 0)]);
    assert!(after.conflicts.is_empty());

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

#[test]
fn selection_matches_core_distribution_select() {
    let root = temp_root("selection-core-cross-check");
    write_selection_dist(&root);

    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    let distribution = sw_core::distribution::Distribution::open(&root).unwrap();

    let pairs = vec![
        hw("CPUBOARD", "IP22"),
        hw("CPUARCH", "R4000"),
        hw("CPUARCH", "R5000"),
    ];
    let mut builder = HardwareProfile::builder();
    for pair in &pairs {
        builder = builder.add(&pair.attribute, &pair.value);
    }
    let core = distribution.select(&builder.build());
    let snapshot = backend.select_entries(pairs).unwrap();

    // Same selection, same order: the bridge adds only key
    // resolution, never its own semantics.
    assert_eq!(snapshot.selected.len(), core.selected.len());
    let bridge_paths: Vec<String> = snapshot
        .selected
        .iter()
        .map(|key| {
            backend
                .entry_detail(key.product_id, key.entry_id)
                .unwrap()
                .path
        })
        .collect();
    let core_paths: Vec<String> = core
        .selected
        .iter()
        .map(|located| located.entry.path.to_string())
        .collect();
    assert_eq!(bridge_paths, core_paths);

    assert_eq!(snapshot.conflicts.len(), core.conflicts.len());
    for (bridge_conflict, core_conflict) in snapshot.conflicts.iter().zip(core.conflicts.iter()) {
        assert_eq!(bridge_conflict.path, core_conflict.path.to_string());
        let bridge_candidates: Vec<String> = bridge_conflict
            .candidates
            .iter()
            .map(|key| {
                backend
                    .entry_detail(key.product_id, key.entry_id)
                    .unwrap()
                    .path
            })
            .collect();
        let core_candidates: Vec<String> = core_conflict
            .candidates
            .iter()
            .map(|located| located.entry.path.to_string())
            .collect();
        assert_eq!(bridge_candidates, core_candidates);
    }

    let _ = std::fs::remove_dir_all(&root);
}

/// Hardware candidate and selection smoke test against a real
/// distribution directory, using the same `SW_EXPLORER_TEST_DIST`
/// opt-in as the other smoke tests.
///
/// The profile is built from values the media actually carry (never
/// from a built-in machine database), and the bridge selection is
/// cross-checked entry by entry against the core selection the CLI
/// exposes through `sw select`.
#[test]
fn real_dist_hardware_smoke() {
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

    // Candidates come from the media: at least one attribute set, no
    // empty names. Empty values are kept when the media carry them
    // (e.g. `GFXBOARD=` restricts an entry to headless boards).
    let candidates = backend.hardware_candidates().unwrap();
    assert!(
        !candidates.is_empty(),
        "expected MACH candidates in a full media set"
    );
    for set in &candidates {
        assert!(!set.attribute.is_empty());
        assert!(!set.values.is_empty());
    }

    // Profile: the first CPUBOARD value the media use (a full media
    // set always restricts some entries by CPU board).
    let board = candidates
        .iter()
        .find(|set| set.attribute == "CPUBOARD")
        .and_then(|set| set.values.first())
        .expect("a full media set carries CPUBOARD candidates");
    let pairs = vec![hw("CPUBOARD", board)];
    let mut builder = HardwareProfile::builder();
    for pair in &pairs {
        builder = builder.add(&pair.attribute, &pair.value);
    }
    let core = distribution.select(&builder.build());
    let snapshot = backend.select_entries(pairs).unwrap();

    assert!(!snapshot.selected.is_empty());
    assert_eq!(snapshot.selected.len(), core.selected.len());
    assert_eq!(snapshot.conflicts.len(), core.conflicts.len());

    // Every selected key resolves back to the very entry the core
    // selected at that position.
    for (key, located) in snapshot.selected.iter().zip(core.selected.iter()) {
        let detail = backend.entry_detail(key.product_id, key.entry_id).unwrap();
        assert_eq!(detail.path, located.entry.path.to_string());
        assert_eq!(detail.subsystem, located.entry.subsystem.to_string());
    }
    // Conflict candidates resolve as well.
    for conflict in &snapshot.conflicts {
        assert!(!conflict.candidates.is_empty());
        for key in &conflict.candidates {
            backend.entry_detail(key.product_id, key.entry_id).unwrap();
        }
    }

    // A different media-carried board selects differently somewhere:
    // at minimum the core cross-check above pins the semantics for
    // any profile, so here only the bridge/core agreement is
    // re-verified for the alternative value.
    if let Some(other) = candidates
        .iter()
        .find(|set| set.attribute == "CPUBOARD")
        .and_then(|set| set.values.get(1))
    {
        let alt_pairs = vec![hw("CPUBOARD", other)];
        let mut builder = HardwareProfile::builder();
        for pair in &alt_pairs {
            builder = builder.add(&pair.attribute, &pair.value);
        }
        let alt_core = distribution.select(&builder.build());
        let alt_snapshot = backend.select_entries(alt_pairs).unwrap();
        assert_eq!(alt_snapshot.selected.len(), alt_core.selected.len());
        assert_eq!(alt_snapshot.conflicts.len(), alt_core.conflicts.len());
    }
}

// ---------------------------------------------------------------------------
// Extraction: request validation, key resolution, planning and execution
// ---------------------------------------------------------------------------

/// Writes one image archive from `(record name, payload)` pairs, in the
/// given order (which must match the IDB order of payload entries).
fn write_archive(root: &Path, product: &str, records: &[(&str, &[u8])]) {
    let mut archive = b"im001V999P00\0".to_vec();
    for (name, payload) in records {
        archive.extend_from_slice(&(name.len() as u16).to_be_bytes());
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(payload);
    }
    std::fs::write(root.join(format!("{product}.sw")), archive).unwrap();
}

/// Writes the extraction test distribution: two IDB-only products with
/// real image archives, plus one product without an archive.
///
/// * `xa` — `bin` dir, `hello.txt`, `bin/tool` x3 (IP22 / IP99 /
///   fallback), explicitly empty `empty.txt`, `old/order.txt` naming a
///   foreign subsystem (its payload record lives in the foreign image
///   archive), `cshrc` with a (fake but plannable) compressed payload.
///   Entry ids 0..=7 in IDB order.
/// * `xb` — `world.txt`. Entry id 0. Its archive also carries the
///   `old/order.txt` record the `xa` IDB references.
/// * `xc` — `xc.txt`, no image archive at all.
fn write_extraction_dist(root: &Path) {
    write_raw_idb(
        root,
        "xa",
        "d 0755 root sys bin src xa.sw.unix\n\
         f 0644 root sys hello.txt src/hello.txt xa.sw.unix sum(1) size(5) cmpsize(0)\n\
         f 0755 root sys bin/tool src/tool1 xa.sw.unix sum(2) size(4) cmpsize(0) mach(CPUBOARD=IP22)\n\
         f 0755 root sys bin/tool src/tool2 xa.sw.unix sum(3) size(4) cmpsize(0) mach(CPUBOARD=IP99)\n\
         f 0755 root sys bin/tool src/tool3 xa.sw.unix sum(4) size(4) cmpsize(0)\n\
         f 0644 root sys empty.txt src/empty xa.sw.unix sum(5) size(0)\n\
         f 0644 root sys old/order.txt src/old xb.sw.unix sum(6) size(4) cmpsize(0)\n\
         f 0644 root sys cshrc src/cshrc xa.sw.unix sum(7) size(10) cmpsize(5)\n",
    );
    write_archive(
        root,
        "xa",
        &[
            ("hello.txt", b"hello".as_slice()),
            ("bin/tool", b"IP22".as_slice()),
            ("bin/tool", b"IP99".as_slice()),
            ("bin/tool", b"FBCK".as_slice()),
            ("cshrc", b"12345".as_slice()),
        ],
    );
    write_idb(root, "xb", &[("xb.sw.unix", "world.txt")]);
    write_archive(
        root,
        "xb",
        &[
            ("world.txt", b"world".as_slice()),
            ("old/order.txt", b"old!".as_slice()),
        ],
    );
    write_idb(root, "xc", &[("xc.sw.unix", "xc.txt")]);
}

/// Builds a Full/Auto/no-overwrite extraction request for the given
/// (product id, entry id) pairs.
fn extraction_request(entries: &[(u64, u64)], out: &Path) -> ffi::ExtractionRequest {
    ffi::ExtractionRequest {
        entries: entries
            .iter()
            .map(|(product_id, entry_id)| ffi::ExtractionEntryKey {
                product_id: *product_id,
                entry_id: *entry_id,
            })
            .collect(),
        hardware: Vec::new(),
        output_dir: out.to_str().unwrap().to_string(),
        path_mode: ffi::ExtractionPathMode::Full,
        relative_to: String::new(),
        decode: ffi::ExtractionDecodeMode::Auto,
        keep_stored: false,
        continue_on_error: true,
        allow_overwrite: false,
    }
}

/// Opens a backend on the extraction test distribution.
fn open_extraction_backend(root: &Path) -> crate::backend::Backend {
    let mut backend = new_backend();
    backend.open_distribution(root.to_str().unwrap()).unwrap();
    *backend
}

#[test]
fn extraction_plan_round_trips_summary() {
    let root = temp_root("ext-plan");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");

    let out = root.join("out");
    let summary = backend
        .plan_extraction(&extraction_request(&[(xa, 1)], &out))
        .unwrap();
    assert_eq!(summary.requested_records, 1);
    assert_eq!(summary.omitted_records, 0);
    assert_eq!(summary.hardware_excluded_records, 0);
    assert_eq!(summary.planned_records, 1);
    assert_eq!(summary.output_paths, 1);
    assert_eq!(summary.existing_outputs, 0);
    // Planning never touches the filesystem.
    assert!(!out.exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_resolves_keys_across_products() {
    let root = temp_root("ext-multi");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");
    let xb = product_object_id(&backend, "xb");

    // A "current view" mixing products: hello.txt from xa plus entry
    // id 0 (perfectly valid) world.txt from xb.
    let out = root.join("out");
    let request = extraction_request(&[(xa, 1), (xb, 0)], &out);
    let summary = backend.plan_extraction(&request).unwrap();
    assert_eq!(summary.planned_records, 2);
    assert_eq!(summary.output_paths, 2);

    let report = backend.extract_entries(&request).unwrap();
    assert_eq!(report.extracted, 2);
    assert!(report.failures.is_empty());
    assert_eq!(std::fs::read(out.join("hello.txt")).unwrap(), b"hello");
    assert_eq!(std::fs::read(out.join("world.txt")).unwrap(), b"world");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_key_resolves_against_the_actual_owning_product() {
    let root = temp_root("ext-owner");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");

    // old/order.txt lives in xa's IDB but names `xb.sw.unix`; it is
    // owned by xa, and the key must resolve against xa — never against
    // a product guessed from the record's subsystem name.
    let detail = backend.entry_detail(xa, 6).unwrap();
    assert_eq!(detail.path, "old/order.txt");
    let out = root.join("out");
    let summary = backend
        .plan_extraction(&extraction_request(&[(xa, 6)], &out))
        .unwrap();
    assert_eq!(summary.planned_records, 1);
    let report = backend
        .extract_entries(&extraction_request(&[(xa, 6)], &out))
        .unwrap();
    assert_eq!(report.extracted, 1);
    assert_eq!(std::fs::read(out.join("old/order.txt")).unwrap(), b"old!");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_duplicate_key_refused() {
    let root = temp_root("ext-dup");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");

    let error = backend
        .plan_extraction(&extraction_request(&[(xa, 1), (xa, 1)], &root.join("out")))
        .err()
        .unwrap();
    assert!(error.to_string().contains("duplicate entry key"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_same_path_under_different_keys_is_legal() {
    let root = temp_root("ext-variants");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");

    // The three bin/tool variants share one path under three different
    // keys; the profile selects exactly one of them.
    let mut request = extraction_request(&[(xa, 2), (xa, 3), (xa, 4)], &root.join("out"));
    request.hardware = vec![hw("CPUBOARD", "IP22")];
    let summary = backend.plan_extraction(&request).unwrap();
    assert_eq!(summary.requested_records, 3);
    assert_eq!(summary.hardware_excluded_records, 2);
    assert_eq!(summary.planned_records, 1);
    assert_eq!(summary.output_paths, 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_rejects_bad_keys() {
    let root = temp_root("ext-badkey");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");

    // Object id 0 is never valid.
    let error = backend
        .plan_extraction(&extraction_request(&[(0, 1)], &root.join("out")))
        .err()
        .unwrap();
    assert!(error.to_string().contains("object id 0"));

    // Unknown object id.
    let error = backend
        .plan_extraction(&extraction_request(&[(999, 1)], &root.join("out")))
        .err()
        .unwrap();
    assert!(error.to_string().contains("does not exist"));

    // A product id pointing at an image instead.
    let image_id = backend
        .hierarchy()
        .unwrap()
        .iter()
        .find(|node| matches!(node.kind, ffi::ObjectKind::Image))
        .map(|node| node.id)
        .unwrap();
    let error = backend
        .plan_extraction(&extraction_request(&[(image_id, 1)], &root.join("out")))
        .err()
        .unwrap();
    assert!(error.to_string().contains("does not identify a product"));

    // Entry id out of range.
    let error = backend
        .plan_extraction(&extraction_request(&[(xa, 999)], &root.join("out")))
        .err()
        .unwrap();
    assert!(error.to_string().contains("does not exist"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_rejects_an_empty_scope() {
    let root = temp_root("ext-empty");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);

    let error = backend
        .plan_extraction(&extraction_request(&[], &root.join("out")))
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "no entries requested");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_validates_the_output_directory() {
    let root = temp_root("ext-outdir");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");

    // Empty output directory.
    let mut request = extraction_request(&[(xa, 1)], &root.join("out"));
    request.output_dir = String::new();
    let error = backend.plan_extraction(&request).err().unwrap();
    assert_eq!(error.to_string(), "output directory is empty");

    // A relative host path is never accepted.
    request.output_dir = "relative/out".to_string();
    let error = backend.plan_extraction(&request).err().unwrap();
    assert!(error.to_string().contains("not an absolute host path"));

    // A nonexistent absolute directory is legitimate and is not
    // created by planning.
    let out = root.join("does-not-exist-yet");
    let request = extraction_request(&[(xa, 1)], &out);
    backend.plan_extraction(&request).unwrap();
    assert!(!out.exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_relative_to_prefix_is_validated_by_the_core_parser() {
    let root = temp_root("ext-relto");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");

    // Full mode ignores the prefix field entirely.
    let mut request = extraction_request(&[(xa, 1)], &root.join("out"));
    request.relative_to = "not a prefix: ../escape".to_string();
    backend.plan_extraction(&request).unwrap();

    // RelativeTo mode hands the prefix to the authoritative parser,
    // which rejects escapes above the root.
    request.path_mode = ffi::ExtractionPathMode::RelativeTo;
    request.relative_to = "../escape".to_string();
    let error = backend.plan_extraction(&request).err().unwrap();
    assert!(error.to_string().contains("invalid relative-to prefix"));

    // A valid prefix strips cleanly: bin/tool becomes tool. A single
    // variant scope is unambiguous even without a profile.
    let mut request = extraction_request(&[(xa, 4)], &root.join("out"));
    request.path_mode = ffi::ExtractionPathMode::RelativeTo;
    request.relative_to = "bin".to_string();
    let summary = backend.plan_extraction(&request).unwrap();
    assert_eq!(summary.planned_records, 1);
    assert_eq!(summary.output_paths, 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_decode_and_option_conversion() {
    let root = temp_root("ext-options");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");

    // Auto decode of a compressed payload: one output path; with
    // keep_stored the `.Z` sidecar adds a second one.
    let request = extraction_request(&[(xa, 7)], &root.join("out"));
    let summary = backend.plan_extraction(&request).unwrap();
    assert_eq!(summary.output_paths, 1);
    let mut request = extraction_request(&[(xa, 7)], &root.join("out"));
    request.keep_stored = true;
    let summary = backend.plan_extraction(&request).unwrap();
    assert_eq!(summary.output_paths, 2);

    // Never decode: the stored bytes go to `cshrc.Z` only.
    let mut request = extraction_request(&[(xa, 7)], &root.join("out"));
    request.decode = ffi::ExtractionDecodeMode::Never;
    let summary = backend.plan_extraction(&request).unwrap();
    assert_eq!(summary.output_paths, 1);

    // Flat mode drops directories: bin/tool lands at out/tool.
    let mut request = extraction_request(&[(xa, 4)], &root.join("out"));
    request.path_mode = ffi::ExtractionPathMode::Flat;
    let out = root.join("out");
    request.output_dir = out.to_str().unwrap().to_string();
    backend.plan_extraction(&request).unwrap();
    let report = backend.extract_entries(&request).unwrap();
    assert_eq!(report.extracted, 1);
    assert_eq!(std::fs::read(out.join("tool")).unwrap(), b"FBCK");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_hardware_pairs_validation() {
    let root = temp_root("ext-hw");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");

    // An empty attribute name is malformed.
    let mut request = extraction_request(&[(xa, 1)], &root.join("out"));
    request.hardware = vec![hw("", "IP22")];
    let error = backend.plan_extraction(&request).err().unwrap();
    assert_eq!(error.to_string(), "hardware attribute name is empty");

    // An empty value is a genuine fact (`GFXBOARD=`).
    request.hardware = vec![hw("CPUBOARD", "IP22"), hw("GFXBOARD", "")];
    backend.plan_extraction(&request).unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_refuse_and_allow_overwrite_policies() {
    let root = temp_root("ext-overwrite");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");

    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("hello.txt"), b"KEEP").unwrap();

    // The GUI default refuses overwrites, before any write.
    let request = extraction_request(&[(xa, 1)], &out);
    let error = backend.plan_extraction(&request).err().unwrap();
    assert!(
        error
            .to_string()
            .contains("extraction would overwrite existing files")
    );
    assert_eq!(std::fs::read(out.join("hello.txt")).unwrap(), b"KEEP");

    // An explicit opt-in allows overwriting existing regular files and
    // reports them in the plan.
    let mut request = extraction_request(&[(xa, 1)], &out);
    request.allow_overwrite = true;
    let summary = backend.plan_extraction(&request).unwrap();
    assert_eq!(summary.existing_outputs, 1);
    let report = backend.extract_entries(&request).unwrap();
    assert_eq!(report.extracted, 1);
    assert_eq!(std::fs::read(out.join("hello.txt")).unwrap(), b"hello");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_runtime_failures_are_report_data_not_errors() {
    let root = temp_root("ext-runtime");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");
    let xb = product_object_id(&backend, "xb");

    // The distribution is committed; then xb's archive disappears, so
    // world.txt fails at read time — a runtime failure, not a refusal.
    std::fs::remove_file(root.join("xb.sw")).unwrap();
    let request = extraction_request(&[(xa, 1), (xb, 0)], &root.join("out"));
    let report = backend.extract_entries(&request).unwrap();
    assert_eq!(report.extracted, 1);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].path, "world.txt");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_report_conversion_surfaces_recoveries() {
    use sw_core::extract::{ExtractFailure, ExtractRecovery, ExtractReport};
    use sw_core::image::PayloadResolution;

    let report = ExtractReport {
        extracted: 2,
        skipped: 1,
        failures: vec![ExtractFailure {
            path: "bad".to_string(),
            message: "broken".to_string(),
        }],
        recoveries: vec![
            ExtractRecovery {
                path: "delta".to_string(),
                expected_record_offset: Some(0x1234),
                actual_record_offset: 0x1240,
                resolution: PayloadResolution::Delta { delta: 12 },
            },
            ExtractRecovery {
                path: "resync".to_string(),
                expected_record_offset: None,
                actual_record_offset: 7,
                resolution: PayloadResolution::Resynced { delta: None },
            },
            ExtractRecovery {
                path: "scan".to_string(),
                expected_record_offset: Some(1),
                actual_record_offset: 2,
                resolution: PayloadResolution::Scanned,
            },
            ExtractRecovery {
                // Never a recovery: exact reads are not reported.
                path: "exact".to_string(),
                expected_record_offset: None,
                actual_record_offset: 0,
                resolution: PayloadResolution::Exact,
            },
        ],
    };
    let detail = crate::extraction::report_detail(report);
    assert_eq!(detail.extracted, 2);
    assert_eq!(detail.skipped, 1);
    assert_eq!(detail.failures.len(), 1);
    assert_eq!(detail.failures[0].message, "broken");
    assert_eq!(detail.recoveries.len(), 3);

    let delta = &detail.recoveries[0];
    assert_eq!(delta.path, "delta");
    assert!(delta.expected_known);
    assert_eq!(delta.expected_offset, 0x1234);
    assert_eq!(delta.actual_offset, 0x1240);
    assert!(matches!(delta.kind, ffi::ExtractionRecoveryKind::Delta));
    assert!(delta.delta_known);
    assert_eq!(delta.delta, 12);

    let resync = &detail.recoveries[1];
    assert!(!resync.expected_known);
    assert!(matches!(resync.kind, ffi::ExtractionRecoveryKind::Resynced));
    assert!(!resync.delta_known);

    let scan = &detail.recoveries[2];
    assert!(matches!(scan.kind, ffi::ExtractionRecoveryKind::Scanned));
}

#[test]
fn extraction_matches_the_core_planner_exactly() {
    let root = temp_root("ext-crosscheck");
    write_extraction_dist(&root);
    let backend = open_extraction_backend(&root);
    let xa = product_object_id(&backend, "xa");

    // The same scope, options and profile through the bridge and
    // through the core directly must agree on every summary count:
    // frontend safety semantics never drift apart.
    let mut request = extraction_request(&[(xa, 1), (xa, 2), (xa, 3), (xa, 4)], &root.join("out"));
    request.hardware = vec![hw("CPUBOARD", "IP22")];
    let bridge_summary = backend.plan_extraction(&request).unwrap();

    let distribution = sw_core::distribution::Distribution::open(&root).unwrap();
    let product_index = distribution
        .products()
        .iter()
        .position(|p| p.name.as_str() == "xa")
        .unwrap();
    let requested: Vec<sw_core::distribution::EntryKey> = (1..=4)
        .map(|id| sw_core::distribution::EntryKey {
            product_index,
            entry_id: sw_core::idb::EntryId(id),
        })
        .collect();
    let profile = HardwareProfile::builder().add("CPUBOARD", "IP22").build();
    let options = sw_core::extract::ExtractOptions {
        existing_output: sw_core::extract::ExistingOutputPolicy::Refuse,
        ..sw_core::extract::ExtractOptions::default()
    };
    let core_plan = sw_core::plan::plan_extraction(
        &distribution,
        &requested,
        &root.join("out"),
        &options,
        Some(&profile),
    )
    .unwrap();

    assert_eq!(
        bridge_summary.requested_records,
        core_plan.summary.requested_records as u64
    );
    assert_eq!(
        bridge_summary.omitted_records,
        core_plan.summary.omitted_records as u64
    );
    assert_eq!(
        bridge_summary.hardware_excluded_records,
        core_plan.summary.hardware_excluded_records as u64
    );
    assert_eq!(
        bridge_summary.planned_records,
        core_plan.summary.planned_records as u64
    );
    assert_eq!(
        bridge_summary.output_paths,
        core_plan.summary.output_paths as u64
    );
    assert_eq!(
        bridge_summary.existing_outputs,
        core_plan.summary.existing_outputs as u64
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_real_dist_smoke() {
    let Some(path) = std::env::var_os("SW_EXPLORER_TEST_DIST") else {
        eprintln!("SW_EXPLORER_TEST_DIST not set; skipping real-dist extraction smoke");
        return;
    };
    let mut backend = new_backend();
    backend.open_distribution(path.to_str().unwrap()).unwrap();

    // Find the first product with entries and take its first row key,
    // exactly as a current-view scope would.
    let hierarchy = backend.hierarchy().unwrap();
    let product_id = hierarchy
        .iter()
        .find(|node| matches!(node.kind, ffi::ObjectKind::Product) && node.entry_count > 0)
        .map(|node| node.id)
        .expect("a product with entries");
    let rows = backend.entries(product_id).unwrap();
    let out = temp_root("ext-real-out");
    let mut planned = None;
    for row in rows.iter().take(50) {
        let request = extraction_request(&[(row.product_id, row.entry_id)], &out);
        if let Ok(summary) = backend.plan_extraction(&request) {
            planned = Some(summary);
            break;
        }
    }
    let summary = planned.expect("a real distribution offers a plannable entry");
    assert_eq!(summary.planned_records, 1);
    assert_eq!(summary.requested_records, 1);
    let _ = std::fs::remove_dir_all(&out);
}
