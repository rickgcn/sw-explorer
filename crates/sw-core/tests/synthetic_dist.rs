//! Deterministic end-to-end tests against a tiny synthetic distribution
//! built in a temporary directory: descriptor + IDB + one image archive.
use std::path::PathBuf;
use sw_core::descriptor::model::{ConditionalFlag, SubsystemPresence, Version};
use sw_core::distribution::Distribution;
use sw_core::extract::{self, DecodeMode, ExtractOptions, PathMode};
use sw_core::image::PayloadResolution;
use sw_core::mach::eval::HardwareProfile;
use sw_core::path::IrixPath;

/// Builds a minimal but complete level-9 descriptor for a product with
/// one `sw` image and one `unix` subsystem, plus — with `ghost` — a
/// second subsystem that no IDB entry references.
fn descriptor_bytes(product: &str, ghost: bool, unix_attrs: &[&str]) -> Vec<u8> {
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
    bytes.extend_from_slice(&1u16.to_be_bytes()); // metadata count
    lp16(&mut bytes, "P16909060_42"); // unknown metadata, kept raw
    bytes.extend_from_slice(&1u16.to_be_bytes()); // image count

    bytes.extend_from_slice(&0x0858u16.to_be_bytes()); // image flags
    lp16(&mut bytes, "sw");
    lp16(&mut bytes, "System Software");
    bytes.extend_from_slice(&0u16.to_be_bytes()); // unknown a
    bytes.extend_from_slice(&9999u16.to_be_bytes()); // order candidate
    bytes.extend_from_slice(&0x0102_0305u32.to_be_bytes()); // version
    bytes.extend_from_slice(&0u32.to_be_bytes()); // reserved
    bytes.extend_from_slice(&0u16.to_be_bytes()); // metadata count
    bytes.extend_from_slice(&if ghost { 2u16 } else { 1u16 }.to_be_bytes());

    // `unix`, with one range in slot 2, one prerequisite clause in
    // slot 3 and one string in slot 7.
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

    if ghost {
        bytes.extend_from_slice(&0x4050u16.to_be_bytes());
        lp16(&mut bytes, "ghost");
        lp16(&mut bytes, "Unshipped Support");
        lp16(&mut bytes, "!noship && test.sw.ghost");
        for _ in 0..9 {
            bytes.extend_from_slice(&0u16.to_be_bytes());
        }
    }
    bytes
}

/// Builds a synthetic `test` product.
///
/// Records (in IDB order), all stored uncompressed:
///
/// ```text
/// hello.txt      "hello"
/// bin/tool       "IP22"           mach(CPUBOARD=IP22)
/// bin/tool       "IP99"           mach(CPUBOARD=IP99)
/// bin/tool       "FBCK"           (no mach)
/// etc/conf       "cfa!"           mach(CPUBOARD=IP22)
/// etc/conf       "cfb!"           mach(CPUARCH=R4400)
/// café.txt       "latin1"         (non-ASCII name)
/// old/order.txt  "old!"           (subsystem appended last)
/// broken         "????"           mach(unparseable)
/// ```
fn build_dist() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("sw-core-synthetic-{}-{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    // Record name bytes are Latin-1: `é` is a single 0xE9 byte.
    let records: Vec<(Vec<u8>, &[u8])> = vec![
        (b"hello.txt".to_vec(), b"hello"),
        (b"bin/tool".to_vec(), b"IP22"),
        (b"bin/tool".to_vec(), b"IP99"),
        (b"bin/tool".to_vec(), b"FBCK"),
        (b"etc/conf".to_vec(), b"cfa!"),
        (b"etc/conf".to_vec(), b"cfb!"),
        (b"caf\xe9.txt".to_vec(), b"latin1"),
        (b"old/order.txt".to_vec(), b"old!"),
        (b"broken".to_vec(), b"????"),
    ];

    let mut archive = b"im001V999P00\0".to_vec();
    assert_eq!(archive.len(), 13);
    for (name, payload) in &records {
        archive.extend_from_slice(&(name.len() as u16).to_be_bytes());
        archive.extend_from_slice(name);
        archive.extend_from_slice(payload);
    }
    std::fs::write(root.join("test.sw"), archive).unwrap();

    std::fs::write(
        root.join("test"),
        descriptor_bytes("test", true, &["DMODE=64bit"]),
    )
    .unwrap();

    // A product whose subsystem carries an unparsable `mach` attribute,
    // and a product with a descriptor but no IDB at all.
    std::fs::write(
        root.join("side"),
        descriptor_bytes("side", false, &["m=IP99"]),
    )
    .unwrap();
    std::fs::write(
        root.join("side.idb"),
        "f 0644 root sys bin/side src/side side.sw.unix sum(1) size(4) cmpsize(0)\n",
    )
    .unwrap();
    std::fs::write(
        root.join("bare"),
        descriptor_bytes("bare", false, &["DMODE=64bit"]),
    )
    .unwrap();

    // Assembled as bytes because the IDB is Latin-1: the café.txt line
    // contains a raw 0xE9 byte.
    let mut idb = b"d 0755 root sys . src test.sw.unix\n\
d 0755 root sys bin src test.sw.unix\n\
f 0644 root sys hello.txt src/hello.txt test.sw.unix sum(1) size(5) cmpsize(0)\n\
f 0755 root sys bin/tool src/tool1 test.sw.unix sum(2) size(4) cmpsize(0) mach(CPUBOARD=IP22)\n\
f 0755 root sys bin/tool src/tool2 test.sw.unix sum(3) size(4) cmpsize(0) mach(CPUBOARD=IP99)\n\
f 0755 root sys bin/tool src/tool3 test.sw.unix sum(4) size(4) cmpsize(0)\n\
f 0644 root sys etc/conf src/confa test.sw.unix sum(5) size(4) cmpsize(0) mach(CPUBOARD=IP22)\n\
f 0644 root sys etc/conf src/confb test.sw.unix sum(6) size(4) cmpsize(0) mach(CPUARCH=R4400)\n\
f 0644 root sys caf"
        .to_vec();
    idb.push(0xE9);
    idb.extend_from_slice(
        b".txt src/cafe test.sw.unix sum(7) size(6) cmpsize(0)\n\
f 0644 root sys old/order.txt src/old sum(8) size(4) cmpsize(0) test.sw.unix\n\
f 0644 root sys broken src/broken test.sw.unix sum(9) size(4) cmpsize(0) mach(=IP22)\n\
l 0777 root sys hello.link src test.sw.unix symval(hello.txt)\n",
    );
    std::fs::write(root.join("test.idb"), idb).unwrap();
    root
}

fn open() -> (PathBuf, Distribution) {
    let root = build_dist();
    let dist = Distribution::open(&root).expect("open synthetic dist");
    (root, dist)
}

#[test]
fn parses_old_and_new_field_orders() {
    let (_root, dist) = open();
    let product = dist.product("test").expect("product test");
    assert_eq!(product.entries.len(), 12);
    assert!(product.descriptor.is_some());

    let old = product
        .entries
        .iter()
        .find(|e| e.path.as_str() == "old/order.txt")
        .expect("old-order entry");
    assert_eq!(old.subsystem.to_string(), "test.sw.unix");
    assert_eq!(old.size(), Some(4));
}

#[test]
fn descriptor_is_the_hierarchy_authority() {
    let (_root, dist) = open();
    let product = dist.product("test").unwrap();
    let descriptor = product.descriptor.as_ref().expect("descriptor");
    assert_eq!(descriptor.layout_level, 9);
    assert_eq!(descriptor.name, "test");
    assert_eq!(descriptor.stamp, 0x0102_0304);
    assert_eq!(product.title.as_deref(), Some("Synthetic Test Product"));
    // Unknown attributes are preserved raw, not interpreted.
    assert_eq!(descriptor.attributes.len(), 1);
    assert_eq!(descriptor.attributes[0].tag, b'P');
    assert_eq!(descriptor.attributes[0].text, "16909060_42");
    assert!(descriptor.attributes[0].mach().is_none());

    let image = product
        .images
        .iter()
        .find(|i| i.name.image() == "sw")
        .expect("image sw");
    assert_eq!(image.title.as_deref(), Some("System Software"));
    assert_eq!(image.version, Some(Version(0x0102_0305)));
    assert_eq!(image.order, Some(9999));

    let unix = image
        .subsystems
        .iter()
        .find(|s| s.name.subsystem() == "unix")
        .expect("subsystem unix");
    assert_eq!(
        unix.presence,
        SubsystemPresence {
            descriptor: true,
            idb: true
        }
    );
    assert_eq!(unix.title.as_deref(), Some("UNIX Kernel"));
    assert_eq!(unix.mapping.as_deref(), Some("EOE"));
    let rules = unix.rules.as_ref().expect("descriptor-decoded rules");
    assert_eq!(rules.prerequisites.len(), 1);
    assert_eq!(rules.prerequisites[0].all_of[0].target(), "other.sw.base");
    assert_eq!(rules.replaces.len(), 1);
    assert_eq!(rules.replaces[0].target(), "patch*.sw.unix");
    assert!(rules.follows.is_empty());
    assert!(rules.incompatibilities.is_empty());
    assert!(rules.updates.is_empty());

    // The raw flag word 0x0852 carries the default bit, and the `D`
    // attribute makes it hardware-conditional.
    let flags = unix.flags.as_ref().expect("descriptor-decoded flags");
    assert_eq!(flags.required, ConditionalFlag::No);
    let ConditionalFlag::When(expressions) = &flags.default else {
        panic!("default flag must be hardware-conditional");
    };
    assert_eq!(expressions.len(), 1);
    assert_eq!(flags.miniroot, ConditionalFlag::No);
    assert!(flags.inplace);
    assert!(!flags.patch);
    assert!(unix.mach.as_ref().unwrap().is_empty());
    assert!(!unix.entry_ids.is_empty());

    // Declared in the descriptor but absent from the IDB: legal, kept,
    // and distinguishable from a shipped subsystem.
    let ghost = image
        .subsystems
        .iter()
        .find(|s| s.name.subsystem() == "ghost")
        .expect("subsystem ghost");
    assert_eq!(
        ghost.presence,
        SubsystemPresence {
            descriptor: true,
            idb: false
        }
    );
    assert!(ghost.entry_ids.is_empty());
}

#[test]
fn reads_payloads_exactly() {
    let (_root, dist) = open();
    let product = dist.product("test").unwrap();
    let mut reader = dist.image_reader();

    let mut checked = 0;
    for entry in &product.entries {
        if entry.payload.is_none() {
            continue;
        }
        let payload = reader.read(entry).expect("read payload");
        assert_eq!(payload.location.resolution, PayloadResolution::Exact);
        assert_eq!(
            payload.decode().unwrap().len() as u64,
            entry.size().unwrap()
        );
        checked += 1;
    }
    assert_eq!(checked, 9);

    // The non-ASCII record name must match byte-exactly.
    let cafe = product
        .entries
        .iter()
        .find(|e| e.path.as_str() == "caf\u{e9}.txt")
        .unwrap();
    let payload = reader.read(cafe).unwrap();
    assert_eq!(payload.bytes, b"latin1");
}

#[test]
fn selection_prefers_matching_specific() {
    let (_root, dist) = open();
    let profile = HardwareProfile::builder()
        .set("CPUBOARD", "IP22")
        .set("CPUARCH", "R4400")
        .build();
    let selection = dist.select(&profile);

    let tools: Vec<_> = selection
        .selected
        .iter()
        .filter(|e| e.entry.path.as_str() == "bin/tool")
        .collect();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].entry.source_path, "src/tool1");

    // Both etc/conf candidates match: conflict reported, both kept.
    let conf = selection
        .conflicts
        .iter()
        .find(|c| c.path.as_str() == "etc/conf")
        .expect("etc/conf conflict");
    assert_eq!(conf.candidates.len(), 2);
    let conf_selected = selection
        .selected
        .iter()
        .filter(|e| e.entry.path.as_str() == "etc/conf")
        .count();
    assert_eq!(conf_selected, 2);

    // The unparseable mach entry is a conflict and never selected.
    assert!(
        selection
            .conflicts
            .iter()
            .any(|c| c.path.as_str() == "broken")
    );
    assert!(
        !selection
            .selected
            .iter()
            .any(|e| e.entry.path.as_str() == "broken")
    );
}

#[test]
fn selection_falls_back_without_match() {
    let (_root, dist) = open();
    let profile = HardwareProfile::builder().set("CPUBOARD", "IP20").build();
    let selection = dist.select(&profile);
    let tools: Vec<_> = selection
        .selected
        .iter()
        .filter(|e| e.entry.path.as_str() == "bin/tool")
        .collect();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].entry.source_path, "src/tool3");
}

#[test]
fn discovers_products_with_only_one_authority() {
    let (_root, dist) = open();
    // Descriptor + IDB.
    assert!(dist.product("test").unwrap().idb_file.is_some());
    assert!(dist.product("side").is_some());

    // Descriptor only: kept, with an empty entry list and a warning.
    let bare = dist.product("bare").expect("descriptor-only product");
    assert!(bare.descriptor.is_some());
    assert!(bare.idb_file.is_none());
    assert!(bare.entries.is_empty());
    assert!(!bare.images.is_empty());
    assert!(
        bare.diagnostics
            .iter()
            .any(|d| d.message.contains("no IDB"))
    );
}

#[test]
fn selection_reports_unresolved_subsystem_mach() {
    let (_root, dist) = open();
    let profile = HardwareProfile::builder()
        .set("CPUBOARD", "IP22")
        .set("CPUARCH", "R4400")
        .build();
    let selection = dist.select(&profile);

    // side.sw.unix carries an unparsable `mach` expression: its
    // applicability is unknown, so its entry is reported as a conflict
    // and never silently selected.
    let side = dist.product("side").unwrap();
    let unix = &side.images[0].subsystems[0];
    assert_eq!(
        unix.mach.as_ref().unwrap().unresolved,
        &["=IP99".to_string()]
    );
    assert!(
        !selection
            .selected
            .iter()
            .any(|e| e.entry.subsystem.to_string() == "side.sw.unix")
    );
    let conflict = selection
        .conflicts
        .iter()
        .find(|c| c.path.as_str() == "bin/side")
        .expect("bin/side conflict");
    assert_eq!(conflict.candidates.len(), 1);
}

#[test]
fn extracts_to_host_filesystem() {
    let (root, dist) = open();
    let out = root.join("out");
    let profile = HardwareProfile::builder().set("CPUBOARD", "IP20").build();
    let selection = dist.select(&profile);
    let mut reader = dist.image_reader();
    let selected: Vec<&sw_core::idb::Entry> = selection
        .selected
        .iter()
        .map(|located| located.entry)
        .collect();

    let report =
        extract::extract_unchecked(&mut reader, &selected, &out, &ExtractOptions::default());
    assert!(report.failures.is_empty(), "{:?}", report.failures);

    assert_eq!(std::fs::read(out.join("hello.txt")).unwrap(), b"hello");
    assert_eq!(std::fs::read(out.join("bin/tool")).unwrap(), b"FBCK");
    assert_eq!(std::fs::read(out.join("caf\u{e9}.txt")).unwrap(), b"latin1");
    assert!(out.join("old").is_dir());

    #[cfg(unix)]
    {
        let link = out.join("hello.link");
        assert_eq!(
            std::fs::read_link(&link).unwrap().to_str().unwrap(),
            "hello.txt"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn regular_entry_without_payload_is_a_failure() {
    let root = std::env::temp_dir().join(format!("sw-core-nopayload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("t"), descriptor_bytes("t", false, &[])).unwrap();
    // No cmpsize at all: no payload record exists for this file.
    std::fs::write(
        root.join("t.idb"),
        "f 0644 root sys ghost src/ghost t.sw.unix sum(1) size(12)\n",
    )
    .unwrap();
    let dist = Distribution::open(&root).unwrap();
    let product = dist.product("t").unwrap();
    let entry = &product.entries[0];
    let mut reader = dist.image_reader();
    let report = extract::extract_unchecked(
        &mut reader,
        &[entry],
        &root.join("out"),
        &ExtractOptions::default(),
    );
    assert_eq!(report.extracted, 0);
    assert_eq!(report.failures.len(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

/// Builds a tiny dist with one genuinely compressed record, using real
/// `.Z` data captured from an IRIX 5.3 distribution.
fn build_compressed_dist() -> PathBuf {
    let compressed = include_bytes!("data/cshrc.z");
    let plain = include_bytes!("data/cshrc.out");

    let root = std::env::temp_dir().join(format!("sw-core-compressed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let mut archive = b"im001V999P00\0".to_vec();
    archive.extend_from_slice(&(b"cshrc".len() as u16).to_be_bytes());
    archive.extend_from_slice(b"cshrc");
    archive.extend_from_slice(compressed);
    std::fs::write(root.join("t.sw"), archive).unwrap();

    std::fs::write(root.join("t"), b"pd001V999P00\0").unwrap();
    std::fs::write(
        root.join("t.idb"),
        format!(
            "f 0644 root sys cshrc src/cshrc t.sw.unix sum(1) size({}) cmpsize({})\n",
            plain.len(),
            compressed.len()
        ),
    )
    .unwrap();
    root
}

#[test]
fn extraction_decode_modes() {
    let compressed = include_bytes!("data/cshrc.z");
    let plain = include_bytes!("data/cshrc.out");

    // Auto: decode in place.
    let root = build_compressed_dist();
    let dist = Distribution::open(&root).unwrap();
    let entry = &dist.product("t").unwrap().entries[0];
    let mut reader = dist.image_reader();
    let out = root.join("out-auto");
    let report =
        extract::extract_unchecked(&mut reader, &[entry], &out, &ExtractOptions::default());
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(std::fs::read(out.join("cshrc")).unwrap(), plain);
    assert!(!out.join("cshrc.Z").exists());

    // Auto + keep_stored: decoded file plus the original .Z bytes.
    let out = root.join("out-keep");
    let options = ExtractOptions {
        keep_stored: true,
        ..ExtractOptions::default()
    };
    let report = extract::extract_unchecked(&mut reader, &[entry], &out, &options);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(std::fs::read(out.join("cshrc")).unwrap(), plain);
    assert_eq!(std::fs::read(out.join("cshrc.Z")).unwrap(), compressed);

    // Never: the stored bytes under a .Z name, no decoded file.
    let out = root.join("out-never");
    let options = ExtractOptions {
        decode: DecodeMode::Never,
        ..ExtractOptions::default()
    };
    let report = extract::extract_unchecked(&mut reader, &[entry], &out, &options);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert!(!out.join("cshrc").exists());
    assert_eq!(std::fs::read(out.join("cshrc.Z")).unwrap(), compressed);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extraction_path_modes() {
    let (root, dist) = open();
    let product = dist.product("test").unwrap();
    let hello: Vec<_> = product
        .entries
        .iter()
        .filter(|e| e.path.as_str() == "hello.txt")
        .collect();
    let tools: Vec<_> = product
        .entries
        .iter()
        .filter(|e| e.path.as_str() == "bin/tool")
        .collect();
    let mut reader = dist.image_reader();

    // Flat: only the file name survives.
    let out = root.join("out-flat");
    let options = ExtractOptions {
        path_mode: PathMode::Flat,
        ..ExtractOptions::default()
    };
    let report = extract::extract_unchecked(&mut reader, &hello, &out, &options);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(std::fs::read(out.join("hello.txt")).unwrap(), b"hello");

    // RelativeTo: the prefix is stripped, outside entries are skipped.
    let out = root.join("out-rel");
    let options = ExtractOptions {
        path_mode: PathMode::RelativeTo(IrixPath::new("bin").unwrap()),
        ..ExtractOptions::default()
    };
    let report = extract::extract_unchecked(&mut reader, &tools, &out, &options);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(report.skipped, 0);
    assert!(out.join("tool").is_file());
    assert!(!out.join("bin/tool").exists());

    // The prefix itself maps to the output root: extracting the `bin`
    // directory entry creates (and applies the mode to) the output
    // directory instead of skipping the entry.
    let bin_dir: Vec<_> = product
        .entries
        .iter()
        .filter(|e| e.path.as_str() == "bin" && e.file_type == sw_core::idb::FileType::Directory)
        .collect();
    assert_eq!(bin_dir.len(), 1);
    let out = root.join("out-root");
    let report = extract::extract_unchecked(&mut reader, &bin_dir, &out, &options);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(report.extracted, 1);
    assert!(out.is_dir());

    let _ = std::fs::remove_dir_all(&root);
}
