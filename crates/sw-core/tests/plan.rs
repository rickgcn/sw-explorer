//! Tests for the shared extraction planner (`sw_core::plan`): ordering
//! invariants, the hardware gates, collision/topology/existing-output
//! preflights, the atomic no-clobber write layer, and checked execution.
use std::path::{Path, PathBuf};
use sw_core::distribution::{Distribution, EntryKey};
use sw_core::error::Error;
use sw_core::extract::{self, DecodeMode, ExistingOutputPolicy, ExtractOptions, PathMode};
use sw_core::idb::Entry;
use sw_core::mach::eval::HardwareProfile;
use sw_core::path::IrixPath;
use sw_core::plan::{self, ExtractionPlan};

const CSHRC_Z: &[u8] = include_bytes!("data/cshrc.z");
const CSHRC_OUT: &[u8] = include_bytes!("data/cshrc.out");

/// Builds a minimal level-9 descriptor for a product with one `sw`
/// image and one `unix` subsystem, with raw attribute blobs per level.
fn descriptor_bytes(
    product: &str,
    product_attrs: &[&str],
    image_attrs: &[&str],
    unix_attrs: &[&str],
) -> Vec<u8> {
    fn lp16(bytes: &mut Vec<u8>, s: &str) {
        bytes.extend_from_slice(&(s.len() as u16).to_be_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }
    fn range(bytes: &mut Vec<u8>, target: (&str, &str, &str), low: u32, high: u32) {
        lp16(bytes, target.0);
        lp16(bytes, target.1);
        lp16(bytes, target.2);
        bytes.extend_from_slice(&low.to_be_bytes());
        bytes.extend_from_slice(&high.to_be_bytes());
    }

    let mut bytes = b"pd001V999P00\0".to_vec();
    for word in [0x07c4u16, 0x0001, 0x07c3, 9] {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
    lp16(&mut bytes, product);
    lp16(&mut bytes, "Synthetic Test Product");
    bytes.extend_from_slice(&0x0850u16.to_be_bytes()); // product flags
    bytes.extend_from_slice(&0x0102_0304u32.to_be_bytes()); // stamp
    bytes.extend_from_slice(&0u32.to_be_bytes()); // reserved
    bytes.extend_from_slice(&(product_attrs.len() as u16).to_be_bytes());
    for attr in product_attrs {
        lp16(&mut bytes, attr);
    }
    bytes.extend_from_slice(&1u16.to_be_bytes()); // image count

    bytes.extend_from_slice(&0x0858u16.to_be_bytes()); // image flags
    lp16(&mut bytes, "sw");
    lp16(&mut bytes, "System Software");
    bytes.extend_from_slice(&0u16.to_be_bytes()); // unknown a
    bytes.extend_from_slice(&9999u16.to_be_bytes()); // order candidate
    bytes.extend_from_slice(&0x0102_0305u32.to_be_bytes()); // version
    bytes.extend_from_slice(&0u32.to_be_bytes()); // reserved
    bytes.extend_from_slice(&(image_attrs.len() as u16).to_be_bytes());
    for attr in image_attrs {
        lp16(&mut bytes, attr);
    }
    bytes.extend_from_slice(&1u16.to_be_bytes()); // subsystem count

    bytes.extend_from_slice(&0x0852u16.to_be_bytes());
    lp16(&mut bytes, "unix");
    lp16(&mut bytes, "UNIX Kernel");
    lp16(&mut bytes, "EOE");
    bytes.extend_from_slice(&0u16.to_be_bytes()); // slot 0
    bytes.extend_from_slice(&0u16.to_be_bytes()); // slot 1
    bytes.extend_from_slice(&1u16.to_be_bytes()); // slot 2: one range
    range(&mut bytes, ("patch*", "sw", "unix"), 0, 0x0102_0304);
    bytes.extend_from_slice(&1u16.to_be_bytes()); // slot 3: one clause
    bytes.extend_from_slice(&1u16.to_be_bytes()); // ... with one range
    range(&mut bytes, ("other", "sw", "base"), 0, 0x7fff_ffff);
    bytes.extend_from_slice(&0u16.to_be_bytes()); // slot 4
    bytes.extend_from_slice(&0u16.to_be_bytes()); // slot 5
    bytes.extend_from_slice(&0u16.to_be_bytes()); // slot 6
    bytes.extend_from_slice(&(unix_attrs.len() as u16).to_be_bytes()); // slot 7
    for attr in unix_attrs {
        lp16(&mut bytes, attr);
    }
    bytes.extend_from_slice(&0u16.to_be_bytes()); // slot 8
    bytes
}

/// Builds an image archive: the 13-byte header, then records in the
/// given order (which must match the IDB order of payload entries).
fn archive_bytes(records: &[(&str, &[u8])]) -> Vec<u8> {
    let mut archive = b"im001V999P00\0".to_vec();
    for (name, payload) in records {
        archive.extend_from_slice(&(name.len() as u16).to_be_bytes());
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(payload);
    }
    archive
}

/// Writes one product: descriptor, IDB and (with records) the `sw`
/// image archive.
fn write_product(
    root: &Path,
    product: &str,
    product_attrs: &[&str],
    image_attrs: &[&str],
    unix_attrs: &[&str],
    idb: &str,
    records: &[(&str, &[u8])],
) {
    std::fs::write(
        root.join(product),
        descriptor_bytes(product, product_attrs, image_attrs, unix_attrs),
    )
    .unwrap();
    std::fs::write(root.join(format!("{product}.idb")), idb).unwrap();
    if !records.is_empty() {
        std::fs::write(root.join(format!("{product}.sw")), archive_bytes(records)).unwrap();
    }
}

/// Builds the synthetic distribution all planner tests share.
///
/// Products:
///
/// ```text
/// test   hello.txt, bin/tool x3 (IP22/IP99/fallback), etc/conf x2
///        (both specific), broken (unparsable mach), cshrc (compressed),
///        empty.txt (explicitly empty), hello.link symlink, dangling
///        symlink (no target), var/pipe FIFO
/// mv     good.txt + bad.txt (mach IP30, no payload record)
/// side   subsystem mach unparsable
/// pun    product-level mach unparsable
/// iun    image-level mach unparsable
/// nb     bin/nb restricted to IP22 only (no fallback)
/// dupa   usr/shared.txt + a/same.txt + only/a.txt + opt/shared dir
/// dupb   usr/shared.txt + b/same.txt + only/b.txt + opt/shared dir
/// zcol   usr/foo (compressed) + a real usr/foo.Z
/// case   usr/Upper.txt + usr/upper.txt, CASEBLK + caseblk/inner,
///        CASEDIR + casedir/inner (host case-folding checks)
/// topo   blk + blk/inner, okdir + okdir/inner, linkdir + linkdir/inner
/// ```
fn build_dist() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("sw-core-plan-{}-{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    write_product(
        &root,
        "test",
        &[],
        &[],
        &[],
        "d 0755 root sys . src test.sw.unix\n\
         d 0755 root sys bin src test.sw.unix\n\
         f 0644 root sys hello.txt src/hello.txt test.sw.unix sum(1) size(5) cmpsize(0)\n\
         f 0755 root sys bin/tool src/tool1 test.sw.unix sum(2) size(4) cmpsize(0) mach(CPUBOARD=IP22)\n\
         f 0755 root sys bin/tool src/tool2 test.sw.unix sum(3) size(4) cmpsize(0) mach(CPUBOARD=IP99)\n\
         f 0755 root sys bin/tool src/tool3 test.sw.unix sum(4) size(4) cmpsize(0)\n\
         f 0644 root sys etc/conf src/confa test.sw.unix sum(5) size(4) cmpsize(0) mach(CPUBOARD=IP22)\n\
         f 0644 root sys etc/conf src/confb test.sw.unix sum(6) size(4) cmpsize(0) mach(CPUARCH=R4400)\n\
         f 0644 root sys broken src/broken test.sw.unix sum(7) size(4) cmpsize(0) mach(=IP22)\n\
         f 0644 root sys cshrc src/cshrc test.sw.unix sum(8) size(CSHRC_SIZE) cmpsize(CSHRC_CSIZE)\n\
         f 0644 root sys empty.txt src/empty test.sw.unix sum(9) size(0)\n\
         l 0777 root sys hello.link src test.sw.unix symval(hello.txt)\n\
         l 0777 root sys dangling src test.sw.unix\n\
         p 0644 root sys var/pipe src test.sw.unix sum(10) size(0)\n"
            .replace("CSHRC_SIZE", &CSHRC_OUT.len().to_string())
            .replace("CSHRC_CSIZE", &CSHRC_Z.len().to_string())
            .as_str(),
        &[
            ("hello.txt", b"hello".as_slice()),
            ("bin/tool", b"IP22".as_slice()),
            ("bin/tool", b"IP99".as_slice()),
            ("bin/tool", b"FBCK".as_slice()),
            ("etc/conf", b"cfa!".as_slice()),
            ("etc/conf", b"cfb!".as_slice()),
            ("broken", b"????".as_slice()),
            ("cshrc", CSHRC_Z),
        ],
    );

    write_product(
        &root,
        "mv",
        &[],
        &[],
        &[],
        "f 0644 root sys good.txt src/good mv.sw.unix sum(1) size(4) cmpsize(0)\n\
         f 0644 root sys bad.txt src/bad mv.sw.unix sum(2) size(4) mach(CPUBOARD=IP30)\n",
        &[("good.txt", b"good".as_slice())],
    );

    write_product(
        &root,
        "side",
        &[],
        &[],
        &["m=IP99"],
        "f 0644 root sys bin/side src/side side.sw.unix sum(1) size(4) cmpsize(0)\n",
        &[("bin/side", b"side".as_slice())],
    );

    write_product(
        &root,
        "pun",
        &["m=IP99"],
        &[],
        &[],
        "f 0644 root sys bin/pun src/pun pun.sw.unix sum(1) size(4) cmpsize(0)\n",
        &[("bin/pun", b"pun!".as_slice())],
    );

    write_product(
        &root,
        "iun",
        &[],
        &["m=IP99"],
        &[],
        "f 0644 root sys bin/iun src/iun iun.sw.unix sum(1) size(4) cmpsize(0)\n",
        &[("bin/iun", b"iun!".as_slice())],
    );

    write_product(
        &root,
        "nb",
        &[],
        &[],
        &[],
        "f 0644 root sys bin/nb src/nb nb.sw.unix sum(1) size(4) cmpsize(0) mach(CPUBOARD=IP22)\n",
        &[("bin/nb", b"nb!!".as_slice())],
    );

    write_product(
        &root,
        "dupa",
        &[],
        &[],
        &[],
        "d 0755 root sys opt/shared src dupa.sw.unix\n\
         f 0644 root sys usr/shared.txt src/shared dupa.sw.unix sum(1) size(4) cmpsize(0)\n\
         f 0644 root sys a/same.txt src/a dupa.sw.unix sum(2) size(4) cmpsize(0)\n\
         f 0644 root sys only/a.txt src/oa dupa.sw.unix sum(3) size(4) cmpsize(0)\n",
        &[
            ("usr/shared.txt", b"dupa".as_slice()),
            ("a/same.txt", b"aaa!".as_slice()),
            ("only/a.txt", b"oa!!".as_slice()),
        ],
    );

    write_product(
        &root,
        "dupb",
        &[],
        &[],
        &[],
        "d 0755 root sys opt/shared src dupb.sw.unix\n\
         f 0644 root sys usr/shared.txt src/shared dupb.sw.unix sum(1) size(4) cmpsize(0)\n\
         f 0644 root sys b/same.txt src/b dupb.sw.unix sum(2) size(4) cmpsize(0)\n\
         f 0644 root sys only/b.txt src/ob dupb.sw.unix sum(3) size(4) cmpsize(0)\n",
        &[
            ("usr/shared.txt", b"dupb".as_slice()),
            ("b/same.txt", b"bbb!".as_slice()),
            ("only/b.txt", b"ob!!".as_slice()),
        ],
    );

    write_product(
        &root,
        "zcol",
        &[],
        &[],
        &[],
        "f 0644 root sys usr/foo src/foo zcol.sw.unix sum(1) size(CSHRC_SIZE) cmpsize(CSHRC_CSIZE)\n\
         f 0644 root sys usr/foo.Z src/foo.Z zcol.sw.unix sum(2) size(4) cmpsize(0)\n"
            .replace("CSHRC_SIZE", &CSHRC_OUT.len().to_string())
            .replace("CSHRC_CSIZE", &CSHRC_Z.len().to_string())
            .as_str(),
        &[("usr/foo", CSHRC_Z), ("usr/foo.Z", b"zz!!".as_slice())],
    );

    write_product(
        &root,
        "case",
        &[],
        &[],
        &[],
        "f 0644 root sys usr/Upper.txt src/upper case.sw.unix sum(1) size(4) cmpsize(0)\n\
         f 0644 root sys usr/upper.txt src/lower case.sw.unix sum(2) size(4) cmpsize(0)\n\
         f 0644 root sys CASEBLK src/caseblk case.sw.unix sum(3) size(4) cmpsize(0)\n\
         f 0644 root sys caseblk/inner src/cbinner case.sw.unix sum(4) size(5) cmpsize(0)\n\
         d 0755 root sys CASEDIR src case.sw.unix\n\
         f 0644 root sys casedir/inner src/cdinner case.sw.unix sum(5) size(5) cmpsize(0)\n",
        &[
            ("usr/Upper.txt", b"UP!!".as_slice()),
            ("usr/upper.txt", b"low!".as_slice()),
            ("CASEBLK", b"blk!".as_slice()),
            ("caseblk/inner", b"cbinn".as_slice()),
            ("casedir/inner", b"cdinn".as_slice()),
        ],
    );

    write_product(
        &root,
        "topo",
        &[],
        &[],
        &[],
        "f 0644 root sys blk src/blk topo.sw.unix sum(1) size(4) cmpsize(0)\n\
         f 0644 root sys blk/inner src/inner topo.sw.unix sum(2) size(5) cmpsize(0)\n\
         d 0755 root sys okdir src topo.sw.unix\n\
         f 0644 root sys okdir/inner src/okinner topo.sw.unix sum(3) size(5) cmpsize(0)\n\
         l 0777 root sys linkdir src topo.sw.unix symval(/tmp/outside)\n\
         f 0644 root sys linkdir/inner src/linner topo.sw.unix sum(4) size(5) cmpsize(0)\n",
        &[
            ("blk", b"blk!".as_slice()),
            ("blk/inner", b"inner".as_slice()),
            ("okdir/inner", b"okinn".as_slice()),
            ("linkdir/inner", b"linne".as_slice()),
        ],
    );

    root
}

fn open() -> (PathBuf, Distribution) {
    let root = build_dist();
    let dist = Distribution::open(&root).expect("open synthetic dist");
    (root, dist)
}

fn keys(dist: &Distribution, product: &str) -> Vec<EntryKey> {
    let product_index = dist
        .products()
        .iter()
        .position(|p| p.name.as_str() == product)
        .unwrap();
    dist.product(product)
        .unwrap()
        .entries
        .iter()
        .map(|entry| EntryKey {
            product_index,
            entry_id: entry.id,
        })
        .collect()
}

fn keys_named(dist: &Distribution, product: &str, path: &str) -> Vec<EntryKey> {
    keys(dist, product)
        .into_iter()
        .filter(|key| dist.entry(*key).unwrap().entry.path.as_str() == path)
        .collect()
}

fn profile(pairs: &[(&str, &str)]) -> HardwareProfile {
    let mut builder = HardwareProfile::builder();
    for (attribute, value) in pairs {
        builder = builder.add(attribute, value);
    }
    builder.build()
}

fn refuse_options() -> ExtractOptions {
    ExtractOptions {
        existing_output: ExistingOutputPolicy::Refuse,
        ..ExtractOptions::default()
    }
}

/// The plain entry references of a key list, for the tests that
/// exercise the low-level writer directly.
fn plain<'a>(dist: &'a Distribution, keys: &[EntryKey]) -> Vec<&'a Entry> {
    keys.iter()
        .map(|key| dist.entry(*key).expect("key resolves").entry)
        .collect()
}

fn plan_err(
    dist: &Distribution,
    requested: &[EntryKey],
    out: &Path,
    options: &ExtractOptions,
    profile: Option<&HardwareProfile>,
) -> String {
    plan::plan_extraction(dist, requested, out, options, profile)
        .expect_err("plan must be refused")
        .to_string()
}

// ---------------------------------------------------------------------
// Ordering: projection before the gate, errors propagated after it.
// ---------------------------------------------------------------------

#[test]
fn projection_error_of_unselected_entry_does_not_block() {
    let (root, dist) = open();
    let requested = keys(&dist, "mv");
    let ip22 = profile(&[("CPUBOARD", "IP22")]);
    let plan = plan::plan_extraction(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        Some(&ip22),
    )
    .expect("bad variant is excluded, so its projection error must not block");
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].entry.path.as_str(), "good.txt");
    assert_eq!(plan.summary.hardware_excluded_records, 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn projection_error_of_selected_entry_blocks() {
    let (root, dist) = open();
    let requested = keys(&dist, "mv");
    let ip30 = profile(&[("CPUBOARD", "IP30")]);
    let error = plan::plan_extraction(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        Some(&ip30),
    )
    .expect_err("the selected bad variant must abort before any write");
    // Static projection failures keep their original error type.
    assert!(matches!(error, Error::PayloadNotFound { .. }));
    assert!(error.to_string().contains("bad.txt"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn gate_runs_before_projection_errors_propagate() {
    let (root, dist) = open();
    // An ambiguous scope (three bin/tool variants) that also contains a
    // statically broken entry: without a profile the ambiguity must be
    // reported, not the payload problem.
    let mut requested = keys_named(&dist, "test", "bin/tool");
    requested.extend(keys_named(&dist, "mv", "bad.txt"));
    let message = plan_err(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        None,
    );
    assert!(message.contains("extraction is ambiguous"), "{message}");
    assert!(!message.contains("payload"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// Deliberate omissions never reach the gate.
// ---------------------------------------------------------------------

#[test]
fn omissions_outside_relative_to_prefix_do_not_participate() {
    let (root, dist) = open();
    // The etc/conf pair would be ambiguous without a profile, but it is
    // outside the `bin` prefix and therefore omitted entirely.
    let mut requested = keys_named(&dist, "test", "etc/conf");
    requested.extend(
        keys_named(&dist, "test", "bin/tool")
            .into_iter()
            .filter(|key| dist.entry(*key).unwrap().entry.source_path == "src/tool3"),
    );
    let options = ExtractOptions {
        path_mode: PathMode::RelativeTo(IrixPath::new("bin").unwrap()),
        ..ExtractOptions::default()
    };
    let plan = plan::plan_extraction(&dist, &requested, &root.join("out"), &options, None)
        .expect("omitted entries must not participate in the gate");
    assert_eq!(plan.summary.requested_records, 3);
    assert_eq!(plan.summary.omitted_records, 2);
    assert_eq!(plan.summary.planned_records, 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn devices_and_unresolved_symlinks_are_omitted() {
    let (root, dist) = open();
    let mut requested = keys_named(&dist, "test", "var/pipe");
    requested.extend(keys_named(&dist, "test", "dangling"));
    requested.extend(keys_named(&dist, "test", "hello.txt"));
    let plan = plan::plan_extraction(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        None,
    )
    .expect("a FIFO and a targetless symlink are deliberate omissions");
    assert_eq!(plan.summary.omitted_records, 2);
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].entry.path.as_str(), "hello.txt");
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// The no-profile gate.
// ---------------------------------------------------------------------

#[test]
fn duplicate_non_directory_paths_refuse_without_profile() {
    let (root, dist) = open();
    let requested = keys_named(&dist, "test", "bin/tool");
    let message = plan_err(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        None,
    );
    assert!(message.contains("extraction is ambiguous"), "{message}");
    assert!(
        message.contains("bin/tool has multiple applicable variants."),
        "{message}"
    );
    assert!(message.contains("Specify a hardware profile."), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn duplicate_directories_are_legal_without_profile() {
    let (root, dist) = open();
    let mut requested = keys_named(&dist, "dupa", "opt/shared");
    requested.extend(keys_named(&dist, "dupb", "opt/shared"));
    requested.extend(keys_named(&dist, "dupa", "only/a.txt"));
    requested.extend(keys_named(&dist, "dupb", "only/b.txt"));
    let plan = plan::plan_extraction(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        None,
    )
    .expect("several subsystems legitimately create the same directory");
    assert_eq!(plan.entries.len(), 4);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unparsable_entry_mach_refuses_without_profile() {
    let (root, dist) = open();
    let requested = keys_named(&dist, "test", "broken");
    let message = plan_err(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        None,
    );
    assert!(message.contains("extraction is ambiguous"), "{message}");
    assert!(message.contains("unparsable mach expression"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unresolved_hierarchy_refuses_without_profile() {
    let (root, dist) = open();
    // Subsystem-, product- and image-level undecodable restrictions all
    // make the affected entries' applicability unknown.
    for product in ["side", "pun", "iun"] {
        let requested = keys(&dist, product);
        let message = plan_err(
            &dist,
            &requested,
            &root.join("out"),
            &ExtractOptions::default(),
            None,
        );
        assert!(
            message.contains("undecodable descriptor hardware restriction"),
            "{product}: {message}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// The hardware-profile gate.
// ---------------------------------------------------------------------

#[test]
fn profile_selects_matching_variant() {
    let (root, dist) = open();
    let requested = keys_named(&dist, "test", "bin/tool");
    let ip22 = profile(&[("CPUBOARD", "IP22")]);
    let plan = plan::plan_extraction(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        Some(&ip22),
    )
    .expect("one specific variant applies");
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].entry.source_path, "src/tool1");
    assert_eq!(plan.summary.hardware_excluded_records, 2);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn profile_falls_back_without_match() {
    let (root, dist) = open();
    let requested = keys_named(&dist, "test", "bin/tool");
    let ip20 = profile(&[("CPUBOARD", "IP20")]);
    let plan = plan::plan_extraction(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        Some(&ip20),
    )
    .expect("the mach-less fallback applies");
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].entry.source_path, "src/tool3");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn conflict_touching_scope_refuses() {
    let (root, dist) = open();
    let requested = keys_named(&dist, "test", "etc/conf");
    let both = profile(&[("CPUBOARD", "IP22"), ("CPUARCH", "R4400")]);
    let message = plan_err(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        Some(&both),
    );
    assert!(message.contains("extraction is ambiguous"), "{message}");
    assert!(message.contains("CONFLICT etc/conf"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn conflict_outside_scope_does_not_block() {
    let (root, dist) = open();
    // etc/conf (both-specific) and broken (unparsable) are conflicts in
    // this profile, but they are outside the requested scope.
    let requested = keys_named(&dist, "test", "hello.txt");
    let both = profile(&[("CPUBOARD", "IP22"), ("CPUARCH", "R4400")]);
    let plan = plan::plan_extraction(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        Some(&both),
    )
    .expect("conflicts outside the scope must not block");
    assert_eq!(plan.entries.len(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn profile_selecting_nothing_refuses() {
    let (root, dist) = open();
    let requested = keys(&dist, "nb");
    let ip99 = profile(&[("CPUBOARD", "IP99")]);
    let message = plan_err(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        Some(&ip99),
    );
    assert!(
        message.contains("no entries in scope apply to the given hardware profile"),
        "{message}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// Output collisions.
// ---------------------------------------------------------------------

#[test]
fn cross_subsystem_collision_refuses() {
    let (root, dist) = open();
    // With a profile both mach-less variants stay selected, and the
    // shared output path is caught by the collision preflight.
    let mut requested = keys(&dist, "dupa");
    requested.extend(keys(&dist, "dupb"));
    let ip22 = profile(&[("CPUBOARD", "IP22")]);
    let message = plan_err(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        Some(&ip22),
    );
    assert!(
        message.contains("extraction would overwrite output files"),
        "{message}"
    );
    assert!(message.contains("usr/shared.txt"), "{message}");
    assert!(message.contains("dupa.sw.unix"), "{message}");
    assert!(message.contains("dupb.sw.unix"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn flat_basename_collision_refuses() {
    let (root, dist) = open();
    let mut requested = keys_named(&dist, "dupa", "a/same.txt");
    requested.extend(keys_named(&dist, "dupb", "b/same.txt"));
    let options = ExtractOptions {
        path_mode: PathMode::Flat,
        ..ExtractOptions::default()
    };
    let message = plan_err(&dist, &requested, &root.join("out"), &options, None);
    assert!(
        message.contains("extraction would overwrite output files"),
        "{message}"
    );
    assert!(message.contains("same.txt"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn raw_mode_z_sidecar_collision_refuses() {
    let (root, dist) = open();
    let requested = keys(&dist, "zcol");
    let options = ExtractOptions {
        decode: DecodeMode::Never,
        ..ExtractOptions::default()
    };
    let message = plan_err(&dist, &requested, &root.join("out"), &options, None);
    assert!(
        message.contains("extraction would overwrite output files"),
        "{message}"
    );
    assert!(message.contains("foo.Z"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn keep_stored_z_sidecar_collision_refuses() {
    let (root, dist) = open();
    let requested = keys(&dist, "zcol");
    let options = ExtractOptions {
        keep_stored: true,
        ..ExtractOptions::default()
    };
    let message = plan_err(&dist, &requested, &root.join("out"), &options, None);
    assert!(
        message.contains("extraction would overwrite output files"),
        "{message}"
    );
    assert!(message.contains("foo.Z"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn auto_decode_without_keep_stored_has_no_sidecar_collision() {
    let (root, dist) = open();
    let requested = keys(&dist, "zcol");
    let plan = plan::plan_extraction(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        None,
    )
    .expect("decoded foo and real foo.Z are distinct outputs");
    assert_eq!(plan.summary.output_paths, 2);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// Output topology.
// ---------------------------------------------------------------------

#[test]
fn non_directory_planned_ancestor_refuses() {
    let (root, dist) = open();
    let requested = keys_named(&dist, "topo", "blk")
        .into_iter()
        .chain(keys_named(&dist, "topo", "blk/inner"))
        .collect::<Vec<_>>();
    let message = plan_err(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        None,
    );
    assert!(
        message.contains("extraction would write through a non-directory output"),
        "{message}"
    );
    // The message renders host output paths, whose separators are
    // platform-native.
    let inner = Path::new("blk").join("inner");
    assert!(message.contains(&inner.display().to_string()), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn directory_planned_ancestor_is_legal() {
    let (root, dist) = open();
    let requested = keys_named(&dist, "topo", "okdir")
        .into_iter()
        .chain(keys_named(&dist, "topo", "okdir/inner"))
        .collect::<Vec<_>>();
    plan::plan_extraction(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        None,
    )
    .expect("a planned directory ancestor is the normal case");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn planned_symlink_ancestor_refuses() {
    let (root, dist) = open();
    let requested = keys_named(&dist, "topo", "linkdir")
        .into_iter()
        .chain(keys_named(&dist, "topo", "linkdir/inner"))
        .collect::<Vec<_>>();
    let message = plan_err(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        None,
    );
    assert!(
        message.contains("extraction would write through a non-directory output"),
        "{message}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn non_directory_mapping_to_output_root_refuses() {
    let (root, dist) = open();
    let requested = keys_named(&dist, "test", "hello.txt");
    let options = ExtractOptions {
        path_mode: PathMode::RelativeTo(IrixPath::new("hello.txt").unwrap()),
        ..ExtractOptions::default()
    };
    let message = plan_err(&dist, &requested, &root.join("out"), &options, None);
    assert!(message.contains("invalid output topology"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// Host case-folding: Windows volumes are case-insensitive, so planned
// outputs differing only in letter case name the same object there.
// ---------------------------------------------------------------------

#[cfg(windows)]
#[test]
fn case_only_output_difference_collides_on_windows() {
    let (root, dist) = open();
    let mut requested = keys_named(&dist, "case", "usr/Upper.txt");
    requested.extend(keys_named(&dist, "case", "usr/upper.txt"));
    let message = plan_err(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        None,
    );
    assert!(
        message.contains("extraction would overwrite output files"),
        "{message}"
    );
    // The message keeps the original (first-seen) path spelling.
    assert!(message.contains("usr/Upper.txt"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(windows)]
#[test]
fn case_only_non_directory_ancestor_blocks_topology_on_windows() {
    let (root, dist) = open();
    let requested = keys_named(&dist, "case", "CASEBLK")
        .into_iter()
        .chain(keys_named(&dist, "case", "caseblk/inner"))
        .collect::<Vec<_>>();
    let message = plan_err(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        None,
    );
    assert!(
        message.contains("extraction would write through a non-directory output"),
        "{message}"
    );
    // The message keeps the original path spelling.
    assert!(message.contains("CASEBLK"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(windows)]
#[test]
fn case_only_directory_ancestor_is_legal_on_windows() {
    let (root, dist) = open();
    let requested = keys_named(&dist, "case", "CASEDIR")
        .into_iter()
        .chain(keys_named(&dist, "case", "casedir/inner"))
        .collect::<Vec<_>>();
    let plan = plan::plan_extraction(
        &dist,
        &requested,
        &root.join("out"),
        &ExtractOptions::default(),
        None,
    )
    .expect("a planned directory ancestor stays legal when only the case differs");
    assert_eq!(plan.summary.planned_records, 2);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// Existing host state.
// ---------------------------------------------------------------------

#[test]
fn existing_regular_target_refuses() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("hello.txt"), b"KEEP").unwrap();
    let requested = keys_named(&dist, "test", "hello.txt");
    let message = plan_err(&dist, &requested, &out, &refuse_options(), None);
    assert!(
        message.contains("extraction would overwrite existing files"),
        "{message}"
    );
    assert_eq!(std::fs::read(out.join("hello.txt")).unwrap(), b"KEEP");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn existing_regular_target_allowed_and_counted() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("hello.txt"), b"OLD").unwrap();
    let requested = keys_named(&dist, "test", "hello.txt");
    let plan = plan::plan_extraction(&dist, &requested, &out, &ExtractOptions::default(), None)
        .expect("Allow permits overwriting an existing regular file");
    assert_eq!(plan.summary.existing_outputs, 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn existing_directory_for_directory_entry_is_allowed() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(out.join("bin")).unwrap();
    let requested = keys_named(&dist, "test", "bin");
    plan::plan_extraction(&dist, &requested, &out, &refuse_options(), None)
        .expect("a directory output may merge into an existing real directory");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn existing_directory_for_regular_entry_refuses() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(out.join("hello.txt")).unwrap();
    let requested = keys_named(&dist, "test", "hello.txt");
    // Even the Allow policy never replaces a directory with a file.
    let message = plan_err(&dist, &requested, &out, &ExtractOptions::default(), None);
    assert!(
        message.contains("extraction conflicts with existing filesystem objects"),
        "{message}"
    );
    assert!(message.contains("is an existing directory"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn existing_symlink_target_refuses_even_with_allow() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::os::unix::fs::symlink("/tmp/elsewhere", out.join("hello.txt")).unwrap();
    let requested = keys_named(&dist, "test", "hello.txt");
    let message = plan_err(&dist, &requested, &out, &ExtractOptions::default(), None);
    assert!(message.contains("existing symbolic link"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn existing_regular_target_refuses_planned_symlink_even_with_allow() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("hello.link"), b"KEEP").unwrap();
    let requested = keys_named(&dist, "test", "hello.link");
    // Allow only ever replaces an existing regular file with a
    // regular-file output; a planned symbolic link taking its place
    // would change the filesystem object type, not overwrite contents.
    let message = plan_err(&dist, &requested, &out, &ExtractOptions::default(), None);
    assert!(
        message.contains("extraction conflicts with existing filesystem objects"),
        "{message}"
    );
    assert!(message.contains("hello.link"), "{message}");
    assert_eq!(std::fs::read(out.join("hello.link")).unwrap(), b"KEEP");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn existing_symlink_ancestor_refuses() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::os::unix::fs::symlink("/tmp/elsewhere", out.join("bin")).unwrap();
    let requested: Vec<EntryKey> = keys_named(&dist, "test", "bin/tool")
        .into_iter()
        .filter(|key| dist.entry(*key).unwrap().entry.source_path == "src/tool3")
        .collect();
    let message = plan_err(&dist, &requested, &out, &refuse_options(), None);
    assert!(
        message.contains("extraction would write through an existing non-directory"),
        "{message}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn existing_regular_file_ancestor_refuses() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("bin"), b"not a dir").unwrap();
    let requested: Vec<EntryKey> = keys_named(&dist, "test", "bin/tool")
        .into_iter()
        .filter(|key| dist.entry(*key).unwrap().entry.source_path == "src/tool3")
        .collect();
    let message = plan_err(&dist, &requested, &out, &refuse_options(), None);
    assert!(
        message.contains("extraction would write through an existing non-directory"),
        "{message}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn output_root_existing_as_file_refuses() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::write(&out, b"not a dir").unwrap();
    let requested = keys_named(&dist, "test", "hello.txt");
    let message = plan_err(&dist, &requested, &out, &refuse_options(), None);
    assert!(message.contains("is not a directory"), "{message}");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn output_root_may_itself_be_a_symlink() {
    let (root, dist) = open();
    let real = root.join("real-out");
    std::fs::create_dir_all(&real).unwrap();
    let out = root.join("out-link");
    std::os::unix::fs::symlink(&real, &out).unwrap();
    let requested = keys_named(&dist, "test", "hello.txt");
    // The output root is the user's explicit trust boundary.
    plan::plan_extraction(&dist, &requested, &out, &refuse_options(), None)
        .expect("the output root itself may be a symbolic link");
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// Planning never mutates the filesystem.
// ---------------------------------------------------------------------

/// A recursive snapshot of a directory tree: relative path, kind and
/// file contents, for mutation detection.
fn snapshot(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(base).unwrap().to_path_buf();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            let kind = if metadata.file_type().is_symlink() {
                format!("symlink:{}", std::fs::read_link(&path).unwrap().display())
            } else if metadata.is_dir() {
                "dir".to_string()
            } else {
                format!("file:{:?}", std::fs::read(&path).unwrap())
            };
            out.push(format!("{}|{kind}", relative.display()));
            if metadata.is_dir() {
                walk(base, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    if dir.exists() {
        walk(dir, dir, &mut out);
    }
    out.sort();
    out
}

#[test]
fn planning_never_mutates_the_filesystem() {
    let (root, dist) = open();
    let playground = root.join("playground");
    std::fs::create_dir_all(playground.join("sub")).unwrap();
    std::fs::write(playground.join("sub/stay.txt"), b"STAY").unwrap();
    let before = snapshot(&playground);

    // A refused plan (cross-subsystem collision) against a nonexistent
    // output directory.
    let mut requested = keys(&dist, "dupa");
    requested.extend(keys(&dist, "dupb"));
    let ip22 = profile(&[("CPUBOARD", "IP22")]);
    assert!(
        plan::plan_extraction(
            &dist,
            &requested,
            &playground.join("new-out"),
            &ExtractOptions::default(),
            Some(&ip22),
        )
        .is_err()
    );
    // A successful plan against another nonexistent output directory.
    let hello = keys_named(&dist, "test", "hello.txt");
    plan::plan_extraction(
        &dist,
        &hello,
        &playground.join("other-out"),
        &refuse_options(),
        None,
    )
    .expect("single-entry plan succeeds");

    assert_eq!(snapshot(&playground), before);
    assert!(!playground.join("new-out").exists());
    assert!(!playground.join("other-out").exists());
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// The atomic no-clobber write layer (second line of defense below the
// planner, exercised through the low-level extract directly).
// ---------------------------------------------------------------------

#[test]
fn refuse_write_never_clobbers_an_existing_regular_file() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("hello.txt"), b"KEEP").unwrap();

    let hello = keys_named(&dist, "test", "hello.txt");
    let mut reader = dist.image_reader();
    let report =
        extract::extract_unchecked(&mut reader, &plain(&dist, &hello), &out, &refuse_options());
    assert_eq!(report.extracted, 0);
    assert_eq!(report.failures.len(), 1);
    // The target was neither truncated nor modified.
    assert_eq!(std::fs::read(out.join("hello.txt")).unwrap(), b"KEEP");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn allow_write_still_overwrites() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("hello.txt"), b"OLD").unwrap();

    let hello = keys_named(&dist, "test", "hello.txt");
    let mut reader = dist.image_reader();
    let report = extract::extract_unchecked(
        &mut reader,
        &plain(&dist, &hello),
        &out,
        &ExtractOptions::default(),
    );
    assert_eq!(report.extracted, 1);
    assert_eq!(std::fs::read(out.join("hello.txt")).unwrap(), b"hello");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn refuse_symlink_never_replaces_an_existing_path() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("hello.link"), b"KEEP").unwrap();

    let link = keys_named(&dist, "test", "hello.link");
    let mut reader = dist.image_reader();
    let report =
        extract::extract_unchecked(&mut reader, &plain(&dist, &link), &out, &refuse_options());
    assert_eq!(report.extracted, 0);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(std::fs::read(out.join("hello.link")).unwrap(), b"KEEP");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn allow_symlink_never_replaces_an_existing_regular_file() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("hello.link"), b"KEEP").unwrap();

    let link = keys_named(&dist, "test", "hello.link");
    let mut reader = dist.image_reader();
    // The Allow policy overwrites regular-file contents only; it never
    // removes an existing file to make room for a symbolic link.
    let report = extract::extract_unchecked(
        &mut reader,
        &plain(&dist, &link),
        &out,
        &ExtractOptions::default(),
    );
    assert_eq!(report.extracted, 0);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(std::fs::read(out.join("hello.link")).unwrap(), b"KEEP");
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn allow_symlink_never_replaces_an_existing_symlink() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::os::unix::fs::symlink("/tmp/elsewhere", out.join("hello.link")).unwrap();

    let link = keys_named(&dist, "test", "hello.link");
    let mut reader = dist.image_reader();
    // A pre-existing link (even a dangling one) is never retargeted.
    let report = extract::extract_unchecked(
        &mut reader,
        &plain(&dist, &link),
        &out,
        &ExtractOptions::default(),
    );
    assert_eq!(report.extracted, 0);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(
        std::fs::read_link(out.join("hello.link")).unwrap(),
        PathBuf::from("/tmp/elsewhere")
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// Checked execution.
// ---------------------------------------------------------------------

#[test]
fn extract_checked_plans_and_extracts() {
    let (root, dist) = open();
    let mut requested = keys_named(&dist, "test", "hello.txt");
    requested.extend(keys_named(&dist, "test", "bin"));
    requested.extend(keys_named(&dist, "test", "empty.txt"));
    let out = root.join("out");
    let checked = plan::extract_checked(&dist, &requested, &out, &refuse_options(), None)
        .expect("the fresh plan succeeds");
    assert_eq!(checked.plan.planned_records, 3);
    assert_eq!(checked.plan.output_paths, 3);
    assert_eq!(checked.report.extracted, 3);
    assert!(checked.report.failures.is_empty());
    assert_eq!(std::fs::read(out.join("hello.txt")).unwrap(), b"hello");
    assert!(out.join("bin").is_dir());
    assert_eq!(std::fs::read(out.join("empty.txt")).unwrap(), b"");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extract_checked_replans_immediately_before_writing() {
    let (root, dist) = open();
    let requested = keys_named(&dist, "test", "hello.txt");
    let out = root.join("out");
    // Preflight succeeds while the output directory does not exist yet.
    plan::plan_extraction(&dist, &requested, &out, &refuse_options(), None)
        .expect("preflight succeeds");
    // The host filesystem changes between preflight and execution.
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("hello.txt"), b"KEEP").unwrap();
    let error = plan::extract_checked(&dist, &requested, &out, &refuse_options(), None)
        .expect_err("the re-evaluated plan refuses before any write");
    assert!(
        error
            .to_string()
            .contains("extraction would overwrite existing files"),
        "{error}"
    );
    assert_eq!(std::fs::read(out.join("hello.txt")).unwrap(), b"KEEP");
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// The plan summary.
// ---------------------------------------------------------------------

#[test]
fn summary_counts_every_category() {
    let (root, dist) = open();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("cshrc"), b"OLD").unwrap();

    let mut requested = keys_named(&dist, "test", "hello.txt");
    requested.extend(keys_named(&dist, "test", "cshrc"));
    requested.extend(keys_named(&dist, "test", "var/pipe"));
    requested.extend(keys_named(&dist, "test", "bin/tool"));
    let options = ExtractOptions {
        keep_stored: true,
        ..ExtractOptions::default()
    };
    let ip22 = profile(&[("CPUBOARD", "IP22")]);
    let plan = plan::plan_extraction(&dist, &requested, &out, &options, Some(&ip22))
        .expect("the scope is unambiguous in this profile");
    assert_eq!(plan.summary.requested_records, 6);
    assert_eq!(plan.summary.omitted_records, 1);
    assert_eq!(plan.summary.hardware_excluded_records, 2);
    assert_eq!(plan.summary.planned_records, 3);
    // hello.txt + cshrc + cshrc.Z (keep_stored) + bin/tool.
    assert_eq!(plan.summary.output_paths, 4);
    assert_eq!(plan.summary.existing_outputs, 1);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------
// Real-distribution smoke tests (opt-in via SW_EXPLORER_TEST_DIST).
// ---------------------------------------------------------------------

fn real_dist() -> Option<PathBuf> {
    std::env::var_os("SW_EXPLORER_TEST_DIST").map(PathBuf::from)
}

#[test]
fn real_dist_planner_smoke() {
    let Some(path) = real_dist() else {
        eprintln!("SW_EXPLORER_TEST_DIST not set; skipping real-dist planner smoke");
        return;
    };
    let dist = Distribution::open(&path).expect("open real distribution");
    let ip22 = profile(&[("CPUBOARD", "IP22")]);
    let out = std::env::temp_dir().join(format!("sw-core-plan-real-{}", std::process::id()));
    let mut planned: Option<ExtractionPlan> = None;
    'outer: for (product_index, product) in dist.products().iter().enumerate() {
        for entry in &product.entries {
            if entry.payload.is_none() {
                continue;
            }
            let key = EntryKey {
                product_index,
                entry_id: entry.id,
            };
            if let Ok(plan) =
                plan::plan_extraction(&dist, &[key], &out, &refuse_options(), Some(&ip22))
            {
                planned = Some(plan);
                break 'outer;
            }
        }
    }
    let plan = planned.expect("a real distribution offers at least one plannable entry");
    assert_eq!(plan.summary.requested_records, 1);
    assert_eq!(plan.summary.planned_records, 1);
    assert!(plan.summary.output_paths >= 1);
    // Entry identity survives the planner round-trip: the planned key
    // resolves back to the very same entry.
    let key = plan.entries[0].key;
    let resolved = dist.entry(key).expect("the planned key resolves");
    assert_eq!(resolved.key, key);
    assert!(std::ptr::eq(resolved.entry, plan.entries[0].entry));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn real_dist_checked_extraction_smoke() {
    let Some(path) = real_dist() else {
        eprintln!("SW_EXPLORER_TEST_DIST not set; skipping real-dist extraction smoke");
        return;
    };
    let dist = Distribution::open(&path).expect("open real distribution");
    let out = std::env::temp_dir().join(format!("sw-core-extract-real-{}", std::process::id()));
    let mut extracted = 0;
    'outer: for (product_index, product) in dist.products().iter().enumerate() {
        for entry in &product.entries {
            if extracted >= 3 {
                break;
            }
            // Small regular payloads only, to keep the smoke quick.
            if entry.payload.is_none()
                || entry.file_type != sw_core::idb::FileType::Regular
                || entry.encoded_size().is_none_or(|size| size > 65536)
            {
                continue;
            }
            let key = EntryKey {
                product_index,
                entry_id: entry.id,
            };
            let Ok(checked) = plan::extract_checked(&dist, &[key], &out, &refuse_options(), None)
            else {
                continue;
            };
            if checked.report.failures.is_empty() && checked.report.extracted == 1 {
                extracted += 1;
                continue 'outer;
            }
        }
    }
    assert!(
        extracted >= 1,
        "a real distribution offers at least one extractable small file"
    );
    let _ = std::fs::remove_dir_all(&out);
}
