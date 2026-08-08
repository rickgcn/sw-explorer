//! Deterministic end-to-end tests against a tiny synthetic distribution
//! built in a temporary directory: descriptor + IDB + one image archive.
use std::path::PathBuf;
use sw_core::distribution::Distribution;
use sw_core::extract::{self, DecodeMode, ExtractOptions, PathMode};
use sw_core::image::PayloadResolution;
use sw_core::mach::eval::HardwareProfile;
use sw_core::path::IrixPath;

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

    std::fs::write(root.join("test"), b"pd001V999P00\0").unwrap();

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
        .filter(|e| e.path.as_str() == "bin/tool")
        .collect();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].source_path, "src/tool1");

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
        .filter(|e| e.path.as_str() == "etc/conf")
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
            .any(|e| e.path.as_str() == "broken")
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
        .filter(|e| e.path.as_str() == "bin/tool")
        .collect();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].source_path, "src/tool3");
}

#[test]
fn extracts_to_host_filesystem() {
    let (root, dist) = open();
    let out = root.join("out");
    let profile = HardwareProfile::builder().set("CPUBOARD", "IP20").build();
    let selection = dist.select(&profile);
    let mut reader = dist.image_reader();

    let report = extract::extract(
        &mut reader,
        &selection.selected,
        &out,
        &ExtractOptions::default(),
    );
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
    std::fs::write(root.join("t"), b"pd001V999P00\0").unwrap();
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
    let report = extract::extract(
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
    let report = extract::extract(&mut reader, &[entry], &out, &ExtractOptions::default());
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(std::fs::read(out.join("cshrc")).unwrap(), plain);
    assert!(!out.join("cshrc.Z").exists());

    // Auto + keep_stored: decoded file plus the original .Z bytes.
    let out = root.join("out-keep");
    let options = ExtractOptions {
        keep_stored: true,
        ..ExtractOptions::default()
    };
    let report = extract::extract(&mut reader, &[entry], &out, &options);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(std::fs::read(out.join("cshrc")).unwrap(), plain);
    assert_eq!(std::fs::read(out.join("cshrc.Z")).unwrap(), compressed);

    // Never: the stored bytes under a .Z name, no decoded file.
    let out = root.join("out-never");
    let options = ExtractOptions {
        decode: DecodeMode::Never,
        ..ExtractOptions::default()
    };
    let report = extract::extract(&mut reader, &[entry], &out, &options);
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
    let report = extract::extract(&mut reader, &hello, &out, &options);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(std::fs::read(out.join("hello.txt")).unwrap(), b"hello");

    // RelativeTo: the prefix is stripped, outside entries are skipped.
    let out = root.join("out-rel");
    let options = ExtractOptions {
        path_mode: PathMode::RelativeTo(IrixPath::new("bin").unwrap()),
        ..ExtractOptions::default()
    };
    let report = extract::extract(&mut reader, &tools, &out, &options);
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
    let report = extract::extract(&mut reader, &bin_dir, &out, &options);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(report.extracted, 1);
    assert!(out.is_dir());

    let _ = std::fs::remove_dir_all(&root);
}
