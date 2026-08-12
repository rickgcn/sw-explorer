//! Integration tests for the `sw` binary.
//!
//! Tests run the compiled binary against a synthetic distribution built
//! in a temporary directory (same construction as the `sw-core`
//! synthetic fixture: descriptor + IDB + image archive, all written as
//! bytes because the formats are Latin-1). A few smoke tests run against
//! a real distribution when `SW_EXPLORER_TEST_DIST` points at one.

use assert_cmd::Command;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A `.Z`-compressed payload and its decoded contents.
const CSHRC_Z: &[u8] = include_bytes!("data/cshrc.z");
const CSHRC_OUT: &[u8] = include_bytes!("data/cshrc.out");

/// A `sw` invocation against `dist`.
fn sw(dist: &Path) -> Command {
    let mut command = Command::cargo_bin("sw").expect("binary sw");
    command.arg("-d").arg(dist);
    command
}

/// Runs `sw` and returns (status code, stdout, stderr).
fn run(command: &mut Command) -> (i32, String, String) {
    let output = command.output().expect("run sw");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A unique temporary directory.
fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("sw-cli-test-{tag}-{}-{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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

/// Writes an image archive from `(record name, payload)` pairs.
fn write_archive(path: &Path, records: &[(&[u8], &[u8])]) {
    let mut archive = b"im001V999P00\0".to_vec();
    assert_eq!(archive.len(), 13);
    for (name, payload) in records {
        archive.extend_from_slice(&(name.len() as u16).to_be_bytes());
        archive.extend_from_slice(name);
        archive.extend_from_slice(payload);
    }
    std::fs::write(path, archive).unwrap();
}

/// Builds the synthetic distribution:
///
/// * `test` — descriptor (with an unshipped `ghost` subsystem and a
///   hardware-conditional `default` flag), IDB and archive. Entries
///   include a multi-variant `bin/tool` (two mach-specific + fallback),
///   a both-specific `etc/conf` pair and a `broken` entry whose `mach`
///   attribute does not parse.
/// * `side` — one entry; the subsystem carries an unparsable `mach`
///   attribute blob, so its applicability is unresolvable.
/// * `bare` — descriptor only, no IDB.
/// * `clean` — conflict-free product with a compressed and a plain
///   payload, for extraction tests.
/// * `dupa` / `dupb` — both ship a file at `usr/shared.txt` (different
///   subsystems, so selection sees no conflict, but the output paths
///   collide); `dupa` also ships `a/same.txt` and `b/same.txt`, which
///   collide under `--flat`; both also declare the same `opt/shared`
///   directory, a benign dir/dir duplicate.
/// * `zcol` — ships a compressed `usr/foo` plus a real uncompressed
///   `usr/foo.Z`, which collides with the `.Z` sidecar of `--raw` and
///   `--keep-stored`.
/// * `miss` — one regular file with a payload and one without, so
///   extraction of the product is statically known to fail.
/// * `mv` — like `miss`, but the payload-less entry only applies to
///   IP30, so a `--mach IP22` extraction must not be blocked by it.
/// * `topo` — a regular file `blk` that is also the ancestor of
///   `blk/inner`, and a symbolic link `linkdir` that is the ancestor of
///   `linkdir/inner`: the shared planner's output-topology gate.
fn build_dist() -> PathBuf {
    let root = temp_dir("dist");

    write_archive(
        &root.join("test.sw"),
        &[
            (b"hello.txt", b"hello".as_slice()),
            (b"bin/tool", b"IP22".as_slice()),
            (b"bin/tool", b"IP99".as_slice()),
            (b"bin/tool", b"FBCK".as_slice()),
            (b"etc/conf", b"cfa!".as_slice()),
            (b"etc/conf", b"cfb!".as_slice()),
            (b"caf\xe9.txt", b"latin1".as_slice()),
            (b"old/order.txt", b"old!".as_slice()),
            (b"broken", b"????".as_slice()),
        ],
    );
    std::fs::write(
        root.join("test"),
        descriptor_bytes("test", true, &["DMODE=64bit"]),
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

    write_archive(
        &root.join("clean.sw"),
        &[
            (b"etc/cshrc", CSHRC_Z),
            (b"bin/clean.txt", b"clean!".as_slice()),
        ],
    );
    std::fs::write(root.join("clean"), descriptor_bytes("clean", false, &[])).unwrap();
    std::fs::write(
        root.join("clean.idb"),
        "d 0755 root sys etc src clean.sw.unix\n\
d 0755 root sys bin src clean.sw.unix\n\
f 0644 root sys etc/cshrc src/cshrc clean.sw.unix sum(1) size(601) cmpsize(465)\n\
f 0644 root sys bin/clean.txt src/clean.txt clean.sw.unix sum(2) size(6) cmpsize(0)\n",
    )
    .unwrap();

    write_archive(
        &root.join("dupa.sw"),
        &[
            (b"a/same.txt", b"aaAA".as_slice()),
            (b"b/same.txt", b"bbBB".as_slice()),
            (b"usr/shared.txt", b"dupa".as_slice()),
        ],
    );
    std::fs::write(root.join("dupa"), descriptor_bytes("dupa", false, &[])).unwrap();
    std::fs::write(
        root.join("dupa.idb"),
        "d 0755 root sys opt/shared src dupa.sw.unix\n\
f 0644 root sys a/same.txt src/a dupa.sw.unix sum(1) size(4) cmpsize(0)\n\
f 0644 root sys b/same.txt src/b dupa.sw.unix sum(2) size(4) cmpsize(0)\n\
f 0644 root sys usr/shared.txt src/s dupa.sw.unix sum(3) size(4) cmpsize(0)\n",
    )
    .unwrap();

    write_archive(
        &root.join("dupb.sw"),
        &[(b"usr/shared.txt", b"dupb".as_slice())],
    );
    std::fs::write(root.join("dupb"), descriptor_bytes("dupb", false, &[])).unwrap();
    std::fs::write(
        root.join("dupb.idb"),
        "d 0755 root sys opt/shared src dupb.sw.unix\n\
f 0644 root sys usr/shared.txt src/s dupb.sw.unix sum(1) size(4) cmpsize(0)\n",
    )
    .unwrap();

    write_archive(
        &root.join("zcol.sw"),
        &[(b"usr/foo", CSHRC_Z), (b"usr/foo.Z", b"dot".as_slice())],
    );
    std::fs::write(root.join("zcol"), descriptor_bytes("zcol", false, &[])).unwrap();
    std::fs::write(
        root.join("zcol.idb"),
        "f 0644 root sys usr/foo src/foo zcol.sw.unix sum(1) size(601) cmpsize(465)\n\
f 0644 root sys usr/foo.Z src/foo.Z zcol.sw.unix sum(2) size(3) cmpsize(0)\n",
    )
    .unwrap();

    write_archive(&root.join("miss.sw"), &[(b"good.txt", b"good".as_slice())]);
    std::fs::write(root.join("miss"), descriptor_bytes("miss", false, &[])).unwrap();
    // `bad.txt` has no `cmpsize`, so it never gets a payload locator.
    std::fs::write(
        root.join("miss.idb"),
        "f 0644 root sys good.txt src/good miss.sw.unix sum(1) size(4) cmpsize(0)\n\
f 0644 root sys bad.txt src/bad miss.sw.unix sum(2) size(3)\n",
    )
    .unwrap();

    write_archive(&root.join("mv.sw"), &[(b"good.txt", b"good".as_slice())]);
    std::fs::write(root.join("mv"), descriptor_bytes("mv", false, &[])).unwrap();
    // `bad.txt` has no `cmpsize`, so it never gets a payload locator;
    // its mach expression only applies to IP30.
    std::fs::write(
        root.join("mv.idb"),
        "f 0644 root sys good.txt src/good mv.sw.unix sum(1) size(4) cmpsize(0) mach(CPUBOARD=IP22)\n\
f 0644 root sys bad.txt src/bad mv.sw.unix sum(2) size(3) mach(CPUBOARD=IP30)\n",
    )
    .unwrap();

    write_archive(
        &root.join("topo.sw"),
        &[
            (b"blk", b"blk!".as_slice()),
            (b"blk/inner", b"inner".as_slice()),
            (b"linkdir/inner", b"linne".as_slice()),
        ],
    );
    std::fs::write(root.join("topo"), descriptor_bytes("topo", false, &[])).unwrap();
    // A regular file that is also the ancestor of another output, and a
    // symbolic link that is one: the shared planner's topology gate.
    std::fs::write(
        root.join("topo.idb"),
        "f 0644 root sys blk src/blk topo.sw.unix sum(1) size(4) cmpsize(0)\n\
f 0644 root sys blk/inner src/inner topo.sw.unix sum(2) size(5) cmpsize(0)\n\
l 0777 root sys linkdir src topo.sw.unix symval(/tmp/outside)\n\
f 0644 root sys linkdir/inner src/linner topo.sw.unix sum(3) size(5) cmpsize(0)\n",
    )
    .unwrap();

    root
}

// --- products ---------------------------------------------------------

#[test]
fn products_lists_all_products() {
    let dist = build_dist();
    let (code, stdout, _) = run(sw(&dist).arg("products"));
    assert_eq!(code, 0);
    assert!(stdout.contains("NAME"));
    for name in ["bare", "clean", "side", "test"] {
        assert!(stdout.contains(name), "missing product {name}");
    }
    // `bare` has a descriptor but no IDB.
    let bare = stdout
        .lines()
        .find(|line| line.starts_with("bare"))
        .expect("bare row");
    let fields: Vec<&str> = bare.split_whitespace().collect();
    assert_eq!(fields[1], "Y", "descriptor column");
    assert_eq!(fields[2], "N", "IDB column");
    assert_eq!(fields[5], "0", "entries column");
}

// --- tree -------------------------------------------------------------

#[test]
fn tree_shows_hierarchy() {
    let dist = build_dist();
    let (code, stdout, _) = run(sw(&dist).args(["tree", "test"]));
    assert_eq!(code, 0);
    assert!(stdout.contains("test — Synthetic Test Product"));
    assert!(stdout.contains("test.sw"));
    assert!(stdout.contains("version 16909061"));
    assert!(stdout.contains("order 9999"));
    assert!(stdout.contains("test.sw.unix"));
    assert!(stdout.contains("12 entries"));
    assert!(stdout.contains("default(conditional)"));
    // Declared in the descriptor but absent from the IDB.
    assert!(stdout.contains("test.sw.ghost"));
    assert!(stdout.contains("0 entries"));
}

#[test]
fn tree_descriptor_only_product() {
    let dist = build_dist();
    let (code, stdout, _) = run(sw(&dist).args(["tree", "bare"]));
    assert_eq!(code, 0);
    assert!(stdout.contains("bare — Synthetic Test Product"));
    assert!(stdout.contains("bare.sw.unix"));
}

#[test]
fn tree_unknown_product_fails() {
    let dist = build_dist();
    let (code, _, stderr) = run(sw(&dist).args(["tree", "nope"]));
    assert_eq!(code, 1);
    assert!(stderr.contains("product not found: nope"));
}

// --- show -------------------------------------------------------------

#[test]
fn show_product() {
    let dist = build_dist();
    let (code, stdout, _) = run(sw(&dist).args(["show", "test"]));
    assert_eq!(code, 0);
    assert!(stdout.contains("Product: test"));
    assert!(stdout.contains("Title:   Synthetic Test Product"));
    assert!(stdout.contains("Descriptor: yes"));
    assert!(stdout.contains("IDB:        yes"));
    assert!(stdout.contains("Images:"));
    assert!(stdout.contains("  test.sw"));
}

#[test]
fn show_image() {
    let dist = build_dist();
    let (code, stdout, _) = run(sw(&dist).args(["show", "test.sw"]));
    assert_eq!(code, 0);
    assert!(stdout.contains("Image:   test.sw"));
    assert!(stdout.contains("Title:   System Software"));
    assert!(stdout.contains("Version: 16909061"));
    assert!(stdout.contains("Order:   9999"));
    assert!(stdout.contains("Subsystems:"));
    assert!(stdout.contains("  test.sw.unix"));
}

#[test]
fn show_subsystem() {
    let dist = build_dist();
    let (code, stdout, _) = run(sw(&dist).args(["show", "test.sw.unix"]));
    assert_eq!(code, 0);
    assert!(stdout.contains("Subsystem: test.sw.unix"));
    assert!(stdout.contains("Title:     UNIX Kernel"));
    assert!(stdout.contains("Image:     test.sw"));
    assert!(stdout.contains("Version:   16909061"));
    assert!(stdout.contains("descriptor: yes"));
    assert!(stdout.contains("IDB:        yes"));
    assert!(stdout.contains("default:  when MODE=64bit"));
    assert!(stdout.contains("Mapping:\n  EOE"));
    assert!(stdout.contains("Entries: 12"));
    assert!(stdout.contains("Prerequisites:\n  other.sw.base 0..maxint"));
    assert!(stdout.contains("patch*.sw.unix 0..16909060"));
}

#[test]
fn show_unresolved_mach_is_prominent() {
    let dist = build_dist();
    let (code, stdout, _) = run(sw(&dist).args(["show", "side.sw.unix"]));
    assert_eq!(code, 0);
    assert!(stdout.contains("Unresolved MACH:"));
    assert!(stdout.contains("=IP99"));
}

#[test]
fn show_rejects_four_segments() {
    let dist = build_dist();
    let (code, _, stderr) = run(sw(&dist).args(["show", "a.b.c.d"]));
    assert_eq!(code, 2);
    assert!(stderr.contains("1 to 3 dot-separated segments"));
}

#[test]
fn show_unknown_subsystem_fails() {
    let dist = build_dist();
    let (code, _, stderr) = run(sw(&dist).args(["show", "test.sw.nope"]));
    assert_eq!(code, 1);
    assert!(stderr.contains("subsystem not found: test.sw.nope"));
}

// --- find -------------------------------------------------------------

#[test]
fn find_plain_filename_matches_substring() {
    let dist = build_dist();
    let (code, stdout, _) = run(sw(&dist).args(["find", "hello.txt"]));
    assert_eq!(code, 0);
    let row = stdout
        .lines()
        .find(|line| line.contains("hello.txt"))
        .expect("hello.txt row");
    assert!(row.contains("test"));
    assert!(row.contains("test.sw.unix"));
    assert!(row.contains("file"));
    // The symlink `hello.link` does not contain the substring.
    assert!(!stdout.contains("hello.link"));
}

#[test]
fn find_wildcard_and_substring() {
    let dist = build_dist();
    let (code, substring, _) = run(sw(&dist).args(["find", "tool"]));
    assert_eq!(code, 0);
    assert_eq!(substring.matches("bin/tool").count(), 3);

    let (code, wildcard, _) = run(sw(&dist).args(["find", "bin/*"]));
    assert_eq!(code, 0);
    assert_eq!(wildcard.matches("bin/tool").count(), 3);

    let (code, question, _) = run(sw(&dist).args(["find", "etc/con?"]));
    assert_eq!(code, 0);
    assert_eq!(question.matches("etc/conf").count(), 2);
}

#[test]
fn find_with_mach_uses_full_selection() {
    let dist = build_dist();
    let (code, stdout, _) = run(sw(&dist).args(["find", "tool", "--mach", "IP22"]));
    assert_eq!(code, 0);
    assert!(stdout.contains("CPUBOARD=IP22"));
    assert!(!stdout.contains("IP99"));
    // The mach-specific IP22 variant beats the fallback, so exactly one
    // row remains.
    assert_eq!(stdout.matches("bin/tool").count(), 1);
}

#[test]
fn find_reports_conflicts_without_failing() {
    let dist = build_dist();
    let (code, stdout, stderr) = run(sw(&dist).args([
        "find",
        "etc/conf",
        "--mach",
        "IP22",
        "--mach",
        "CPUARCH=R4400",
    ]));
    assert_eq!(code, 0);
    assert_eq!(stdout.matches("etc/conf").count(), 2);
    assert!(stderr.contains("CONFLICT etc/conf"));
    assert!(stderr.contains("mach: CPUBOARD=IP22"));
    assert!(stderr.contains("mach: CPUARCH=R4400"));
}

#[test]
fn find_no_match_is_not_an_error() {
    let dist = build_dist();
    let (code, _, stderr) = run(sw(&dist).args(["find", "no-such-file"]));
    assert_eq!(code, 0);
    assert!(stderr.contains("no entries match"));
}

#[test]
fn find_rejects_an_empty_query() {
    let dist = build_dist();
    let (code, _, stderr) = run(sw(&dist).args(["find", ""]));
    assert_eq!(code, 1);
    assert!(stderr.contains("search query is empty"), "{stderr}");
}

/// Builds the ownership regression distribution: `alpha`'s IDB carries
/// a record naming the *foreign* subsystem `beta.sw.unix` (plus one
/// record naming its own `alpha.sw.unix`), while `beta` has its own
/// record in `beta.sw.unix`. The foreign record belongs to alpha; the
/// subsystem name is not an ownership boundary.
fn build_foreign_dist() -> PathBuf {
    let root = temp_dir("foreign");
    // Both records are explicitly empty regular files: no image
    // archive is needed for the extraction assertion below.
    std::fs::write(
        root.join("alpha.idb"),
        "f 0755 root sys usr/bin/foreign src/f beta.sw.unix sum(1) size(0) mach(CPUBOARD=IP22)\n\
         f 0644 root sys usr/bin/own src/o alpha.sw.unix sum(2) size(0)\n",
    )
    .unwrap();
    std::fs::write(
        root.join("beta.idb"),
        "f 0755 root sys usr/bin/beta src/b beta.sw.unix sum(1) size(0) mach(CPUBOARD=IP22)\n",
    )
    .unwrap();
    root
}

#[test]
fn find_with_mach_keeps_foreign_records_with_their_actual_owner() {
    let dist = build_foreign_dist();
    let (code, stdout, _) = run(sw(&dist).args(["find", "foreign", "--mach", "CPUBOARD=IP22"]));
    assert_eq!(code, 0);
    let row = stdout
        .lines()
        .find(|line| line.contains("usr/bin/foreign"))
        .expect("the foreign record is found");
    // The displayed product is the actual owner (alpha), never the
    // product segment of the record's subsystem name (beta).
    assert!(row.starts_with("alpha"), "{row}");
    assert!(row.contains("beta.sw.unix"), "{row}");
    // beta's own record shares the entry id but is a different entry:
    // it must not leak into this result.
    assert!(!stdout.contains("usr/bin/beta"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dist);
}

#[test]
fn extract_by_path_keeps_foreign_records_with_their_actual_owner() {
    let dist = build_foreign_dist();
    let out = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args([
            "extract",
            "--path",
            "foreign",
            "--mach",
            "CPUBOARD=IP22",
            "-o",
        ])
        .arg(&out));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(std::fs::read(out.join("usr/bin/foreign")).unwrap(), b"");
    // beta's same-id record was never in the requested scope.
    assert!(!out.join("usr/bin/beta").exists());
    let _ = std::fs::remove_dir_all(&dist);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn tree_shows_foreign_image_under_its_actual_owner() {
    // alpha's tree carries the synthetic foreign image beta.sw: the
    // qualified media name differs from the container, and the tree
    // shows both facts as they are.
    let dist = build_foreign_dist();
    let (code, stdout, _) = run(sw(&dist).args(["tree", "alpha"]));
    assert_eq!(code, 0);
    assert!(stdout.contains("alpha"), "{stdout}");
    assert!(stdout.contains("beta.sw"), "{stdout}");
    assert!(stdout.contains("beta.sw.unix"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dist);
}

#[test]
fn show_ambiguous_image_fails_with_candidates() {
    // Both alpha (synthetic, from the foreign IDB reference) and beta
    // carry a logical beta.sw image: the name alone cannot pick one.
    let dist = build_foreign_dist();
    let (code, _, stderr) = run(sw(&dist).args(["show", "beta.sw"]));
    assert_eq!(code, 1);
    assert!(
        stderr.contains("image name is ambiguous: beta.sw"),
        "{stderr}"
    );
    assert!(stderr.contains("containing product alpha"), "{stderr}");
    assert!(stderr.contains("containing product beta"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dist);
}

#[test]
fn show_ambiguous_subsystem_fails_with_candidates() {
    let dist = build_foreign_dist();
    let (code, _, stderr) = run(sw(&dist).args(["show", "beta.sw.unix"]));
    assert_eq!(code, 1);
    assert!(
        stderr.contains("subsystem name is ambiguous: beta.sw.unix"),
        "{stderr}"
    );
    assert!(stderr.contains("containing product alpha"), "{stderr}");
    assert!(stderr.contains("containing product beta"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dist);
}

#[test]
fn extract_ambiguous_image_refuses_before_writing() {
    let dist = build_foreign_dist();
    // A path that does not exist yet: a refusal must not create it.
    let out = temp_dir("out").join("refused");
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--image", "beta.sw", "-o"])
        .arg(&out));
    assert_eq!(code, 1);
    assert!(
        stderr.contains("image name is ambiguous: beta.sw"),
        "{stderr}"
    );
    assert!(stderr.contains("containing product alpha"), "{stderr}");
    assert!(stderr.contains("containing product beta"), "{stderr}");
    assert!(!out.exists(), "a refused extraction writes nothing");
    let _ = std::fs::remove_dir_all(&dist);
}

#[test]
fn extract_ambiguous_subsystem_refuses_before_writing() {
    let dist = build_foreign_dist();
    let out = temp_dir("out").join("refused");
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--subsystem", "beta.sw.unix", "-o"])
        .arg(&out));
    assert_eq!(code, 1);
    assert!(
        stderr.contains("subsystem name is ambiguous: beta.sw.unix"),
        "{stderr}"
    );
    assert!(stderr.contains("containing product alpha"), "{stderr}");
    assert!(stderr.contains("containing product beta"), "{stderr}");
    assert!(!out.exists(), "a refused extraction writes nothing");
    let _ = std::fs::remove_dir_all(&dist);
}

#[test]
fn extract_unique_image_and_subsystem_scopes_still_work() {
    // alpha.sw / alpha.sw.unix exist only under alpha: unique lookups
    // keep their behavior on unambiguous names.
    let dist = build_foreign_dist();
    let (code, stdout, _) = run(sw(&dist).args(["show", "alpha.sw.unix"]));
    assert_eq!(code, 0);
    assert!(stdout.contains("Subsystem: alpha.sw.unix"), "{stdout}");

    let out = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--subsystem", "alpha.sw.unix", "-o"])
        .arg(&out));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(std::fs::read(out.join("usr/bin/own")).unwrap(), b"");
    let _ = std::fs::remove_dir_all(&dist);
    let _ = std::fs::remove_dir_all(&out);
}

// --- select -----------------------------------------------------------

#[test]
fn select_specific_beats_fallback() {
    let dist = build_dist();
    let (code, stdout, _) = run(sw(&dist).args(["select", "--mach", "IP22", "--list"]));
    assert_eq!(code, 0);
    assert!(stdout.contains("Hardware profile:\n  CPUBOARD = IP22"));
    assert!(stdout.contains("Selected entries: 27"), "{stdout}");
    // `broken` (unparsable mach) and `bin/side` (unresolvable subsystem
    // applicability) can never be selected.
    assert!(stdout.contains("Conflicts:        2"));
    // Only the IP22 variant of bin/tool is selected, not the fallback.
    let tool_lines = stdout.lines().filter(|line| *line == "bin/tool").count();
    assert_eq!(tool_lines, 1);
}

#[test]
fn select_shows_conflicts() {
    let dist = build_dist();
    let (code, stdout, _) =
        run(sw(&dist).args(["select", "--mach", "IP22", "--mach", "CPUARCH=R4400"]));
    assert_eq!(code, 0);
    // Both etc/conf variants match: a genuine conflict.
    assert!(stdout.contains("Conflicts:        3"));
    assert!(stdout.contains("CONFLICT etc/conf"));
    assert!(stdout.contains("mach: CPUBOARD=IP22"));
    assert!(stdout.contains("mach: CPUARCH=R4400"));
}

#[test]
fn select_shows_unresolved_applicability() {
    let dist = build_dist();
    let (code, stdout, _) = run(sw(&dist).args(["select", "--mach", "IP22"]));
    assert_eq!(code, 0);
    assert!(stdout.contains("CONFLICT broken"));
    assert!(stdout.contains("mach: unparsable: =IP22"));
    assert!(stdout.contains("CONFLICT bin/side"));
}

// --- extract ----------------------------------------------------------

#[test]
fn extract_by_path_full_hierarchy() {
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, stdout, _) = run(sw(&dist)
        .args(["extract", "--path", "hello.txt", "-o"])
        .arg(&out));
    assert_eq!(code, 0);
    assert!(stdout.contains("Extracted:  1"));
    assert!(stdout.contains("Errors:     0"));
    assert_eq!(
        std::fs::read(out.join("hello.txt")).unwrap(),
        b"hello".as_slice()
    );
}

#[test]
fn extract_by_subsystem_image_and_product() {
    for scope in [
        vec!["--subsystem", "clean.sw.unix"],
        vec!["--image", "clean.sw"],
        vec!["--product", "clean"],
    ] {
        let dist = build_dist();
        let out = temp_dir("out");
        let mut command = sw(&dist);
        command.arg("extract").args(scope).arg("-o").arg(&out);
        let (code, stdout, _) = run(&mut command);
        assert_eq!(code, 0);
        assert!(stdout.contains("Extracted:  4"));
        // DecodeMode::Auto decodes the compressed payload.
        assert_eq!(std::fs::read(out.join("etc/cshrc")).unwrap(), CSHRC_OUT);
        assert!(!out.join("etc/cshrc.Z").exists());
        assert_eq!(
            std::fs::read(out.join("bin/clean.txt")).unwrap(),
            b"clean!".as_slice()
        );
    }
}

#[test]
fn extract_flat() {
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, _) = run(sw(&dist)
        .args(["extract", "--product", "clean", "--flat", "-o"])
        .arg(&out));
    assert_eq!(code, 0);
    assert!(out.join("cshrc").exists());
    assert!(out.join("clean.txt").exists());
    // Everything lands by file name; nothing below `etc/`.
    assert!(!out.join("etc/cshrc").exists());
}

#[test]
fn extract_relative_to() {
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, stdout, _) = run(sw(&dist)
        .args([
            "extract",
            "--product",
            "clean",
            "--relative-to",
            "etc",
            "-o",
        ])
        .arg(&out));
    assert_eq!(code, 0);
    // Only the subtree below `etc` is extracted; entries outside the
    // prefix produce no output and are excluded up front rather than
    // reported as skipped.
    assert!(stdout.contains("Extracted:  2"));
    assert!(stdout.contains("Skipped:    0"));
    assert_eq!(std::fs::read(out.join("cshrc")).unwrap(), CSHRC_OUT);
    assert!(!out.join("bin").exists());
}

#[test]
fn extract_relative_to_ignores_ambiguity_outside_the_prefix() {
    // The `test` product is full of ambiguities (bin/tool, etc/conf,
    // broken), but none of them is below `old`, so extracting that
    // subtree must work without a hardware profile.
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, stdout, _) = run(sw(&dist)
        .args(["extract", "--product", "test", "--relative-to", "old", "-o"])
        .arg(&out));
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(
        std::fs::read(out.join("order.txt")).unwrap(),
        b"old!".as_slice()
    );
}

#[test]
fn extract_refuses_output_collision_after_mach() {
    // dupa and dupb both ship usr/shared.txt from different subsystems,
    // so selection reports no conflict, yet both would write the same
    // output file.
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--path", "shared.txt", "--mach", "IP22", "-o"])
        .arg(&out));
    assert_eq!(code, 1);
    assert!(stderr.contains("extraction would overwrite output files"));
    assert!(stderr.contains("usr/shared.txt"));
    assert!(stderr.contains("<- dupa.sw.unix: usr/shared.txt"));
    assert!(stderr.contains("<- dupb.sw.unix: usr/shared.txt"));
    assert!(!out.join("usr").exists());
}

#[test]
fn extract_refuses_flat_basename_collision() {
    let dist = build_dist();
    let out = temp_dir("out");
    // Full paths: a/same.txt and b/same.txt coexist fine.
    let (code, _, _) = run(sw(&dist)
        .args(["extract", "--product", "dupa", "-o"])
        .arg(&out));
    assert_eq!(code, 0);
    assert_eq!(
        std::fs::read(out.join("a/same.txt")).unwrap(),
        b"aaAA".as_slice()
    );
    assert_eq!(
        std::fs::read(out.join("b/same.txt")).unwrap(),
        b"bbBB".as_slice()
    );

    // Flat: both would become <out>/same.txt.
    let flat = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--product", "dupa", "--flat", "-o"])
        .arg(&flat));
    assert_eq!(code, 1);
    assert!(stderr.contains("extraction would overwrite output files"));
    assert!(stderr.contains("same.txt"));
    assert!(!flat.join("same.txt").exists());
}

#[test]
fn extract_missing_payload_fails_before_writing() {
    // `bad.txt` has no payload locator: extraction is statically known
    // to fail, so the command must fail before even the valid
    // `good.txt` is written.
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--product", "miss", "-o"])
        .arg(&out));
    assert_eq!(code, 1);
    assert!(stderr.contains("entry has no payload: bad.txt"));
    assert!(!out.join("good.txt").exists());
}

#[test]
fn extract_unselected_missing_payload_does_not_block() {
    // The IP30-only `bad.txt` has no payload, but `--mach IP22`
    // discards it during selection, so it must not abort the
    // extraction of the applicable entries.
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, stdout, stderr) = run(sw(&dist)
        .args(["extract", "--product", "mv", "--mach", "IP22", "-o"])
        .arg(&out));
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert_eq!(std::fs::read(out.join("good.txt")).unwrap(), b"good");
    assert!(!out.join("bad.txt").exists());
}

#[test]
fn extract_selected_missing_payload_fails_before_writing() {
    // With `--mach IP30` the payload-less `bad.txt` is selected instead:
    // the statically known failure must abort the command before
    // anything is written.
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--product", "mv", "--mach", "IP30", "-o"])
        .arg(&out));
    assert_eq!(code, 1);
    assert!(stderr.contains("entry has no payload: bad.txt"));
    assert!(std::fs::read_dir(&out).unwrap().next().is_none());
}

#[test]
fn extract_allows_duplicate_directories_without_mach() {
    // dupa and dupb both declare the `opt/shared` directory; dir/dir
    // duplicates are benign and must not be treated as ambiguity.
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, stdout, _) = run(sw(&dist)
        .args(["extract", "--path", "opt/shared", "-o"])
        .arg(&out));
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("Extracted:  2"));
    assert!(out.join("opt/shared").is_dir());
}

#[test]
fn extract_z_sidecar_collisions() {
    // Auto decode: `usr/foo` decodes next to the real `usr/foo.Z`, no
    // collision.
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, _) = run(sw(&dist)
        .args(["extract", "--product", "zcol", "-o"])
        .arg(&out));
    assert_eq!(code, 0);
    assert_eq!(std::fs::read(out.join("usr/foo")).unwrap(), CSHRC_OUT);
    assert_eq!(
        std::fs::read(out.join("usr/foo.Z")).unwrap(),
        b"dot".as_slice()
    );

    // --raw writes the compressed `usr/foo` as `usr/foo.Z`, overwriting
    // the real one.
    let raw = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--product", "zcol", "--raw", "-o"])
        .arg(&raw));
    assert_eq!(code, 1);
    assert!(stderr.contains("extraction would overwrite output files"));
    assert!(stderr.contains("foo.Z"));
    assert!(!raw.join("usr").exists());

    // --keep-stored writes the same sidecar.
    let kept = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--product", "zcol", "--keep-stored", "-o"])
        .arg(&kept));
    assert_eq!(code, 1);
    assert!(stderr.contains("extraction would overwrite output files"));
    assert!(!kept.join("usr").exists());
}

#[test]
fn extract_raw_keeps_stored_bytes() {
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, _) = run(sw(&dist)
        .args(["extract", "--product", "clean", "--raw", "-o"])
        .arg(&out));
    assert_eq!(code, 0);
    assert_eq!(std::fs::read(out.join("etc/cshrc.Z")).unwrap(), CSHRC_Z);
    assert!(!out.join("etc/cshrc").exists());
    // Uncompressed payloads keep their plain name.
    assert!(out.join("bin/clean.txt").exists());
}

#[test]
fn extract_keep_stored_writes_both() {
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, _) = run(sw(&dist)
        .args(["extract", "--product", "clean", "--keep-stored", "-o"])
        .arg(&out));
    assert_eq!(code, 0);
    assert_eq!(std::fs::read(out.join("etc/cshrc")).unwrap(), CSHRC_OUT);
    assert_eq!(std::fs::read(out.join("etc/cshrc.Z")).unwrap(), CSHRC_Z);
}

#[test]
fn extract_mach_selects_one_variant() {
    for (mach, expected) in [("IP22", "IP22"), ("IP99", "IP99"), ("IP32", "FBCK")] {
        let dist = build_dist();
        let out = temp_dir("out");
        let (code, _, _) = run(sw(&dist)
            .args(["extract", "--path", "bin/tool", "--mach", mach, "-o"])
            .arg(&out));
        assert_eq!(code, 0, "mach {mach}");
        assert_eq!(
            std::fs::read(out.join("bin/tool")).unwrap(),
            expected.as_bytes()
        );
    }
}

#[test]
fn extract_refuses_multiple_variants_without_mach() {
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--path", "bin/tool", "-o"])
        .arg(&out));
    assert_eq!(code, 1);
    assert!(stderr.contains("extraction is ambiguous"));
    assert!(stderr.contains("bin/tool has multiple applicable variants."));
    // The hint comes from the shared sw-core planner, which is
    // frontend-neutral: the CLI's `--mach` flag is not named in it.
    assert!(stderr.contains("Specify a hardware profile."));
    assert!(!out.join("bin").exists());
}

#[test]
fn extract_refuses_remaining_conflict_with_mach() {
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args([
            "extract",
            "--path",
            "etc/conf",
            "--mach",
            "IP22",
            "--mach",
            "CPUARCH=R4400",
            "-o",
        ])
        .arg(&out));
    assert_eq!(code, 1);
    assert!(stderr.contains("extraction is ambiguous"));
    assert!(stderr.contains("CONFLICT etc/conf"));
    assert!(!out.join("etc").exists());
}

#[test]
fn extract_refuses_unparsable_entry_mach() {
    for extra in [vec![], vec!["--mach", "IP22"]] {
        let dist = build_dist();
        let out = temp_dir("out");
        let mut command = sw(&dist);
        command
            .arg("extract")
            .args(["--path", "broken"])
            .args(extra);
        command.arg("-o").arg(&out);
        let (code, _, stderr) = run(&mut command);
        assert_eq!(code, 1);
        assert!(stderr.contains("ambiguous"));
        assert!(!out.join("broken").exists());
    }
}

#[test]
fn extract_refuses_unresolvable_subsystem_applicability() {
    for extra in [vec![], vec!["--mach", "IP22"]] {
        let dist = build_dist();
        let out = temp_dir("out");
        let mut command = sw(&dist);
        command
            .arg("extract")
            .args(["--product", "side"])
            .args(extra);
        command.arg("-o").arg(&out);
        let (code, _, stderr) = run(&mut command);
        assert_eq!(code, 1);
        assert!(stderr.contains("ambiguous"));
        assert!(!out.join("bin").exists());
    }
}

#[test]
fn extract_refuses_non_directory_planned_ancestor() {
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist).args(["extract", "--path", "blk", "-o"]).arg(&out));
    assert_eq!(code, 1);
    assert!(
        stderr.contains("write through a non-directory output"),
        "{stderr}"
    );
    assert!(!out.join("blk").exists());
}

#[cfg(unix)]
#[test]
fn extract_refuses_planned_symlink_ancestor() {
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--path", "linkdir", "-o"])
        .arg(&out));
    assert_eq!(code, 1);
    assert!(
        stderr.contains("write through a non-directory output"),
        "{stderr}"
    );
    assert!(!out.join("linkdir").exists());
}

#[cfg(unix)]
#[test]
fn extract_refuses_existing_symlink_target() {
    let dist = build_dist();
    let out = temp_dir("out");
    std::fs::create_dir_all(out.join("bin")).unwrap();
    std::os::unix::fs::symlink("/tmp/elsewhere", out.join("bin/clean.txt")).unwrap();
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--product", "clean", "-o"])
        .arg(&out));
    assert_eq!(code, 1);
    // A naive overwrite would follow the link outside the output root.
    assert!(stderr.contains("existing symbolic link"), "{stderr}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn extract_refuses_output_root_that_is_a_file() {
    let dist = build_dist();
    let out = temp_dir("out");
    std::fs::create_dir_all(&out).unwrap();
    let file = out.join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--product", "clean", "-o"])
        .arg(&file));
    assert_eq!(code, 1);
    assert!(stderr.contains("is not a directory"), "{stderr}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn extract_requires_exactly_one_scope() {
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, _) = run(sw(&dist).arg("extract").arg("-o").arg(&out));
    assert_eq!(code, 2);
    let (code, _, _) = run(sw(&dist)
        .args(["extract", "--product", "clean", "--image", "clean.sw", "-o"])
        .arg(&out));
    assert_eq!(code, 2);
}

#[test]
fn extract_rejects_invalid_relative_to() {
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args([
            "extract",
            "--product",
            "clean",
            "--relative-to",
            "../escape",
            "-o",
        ])
        .arg(&out));
    assert_eq!(code, 2);
    assert!(stderr.contains("invalid value"));
}

#[test]
fn extract_unknown_product_fails() {
    let dist = build_dist();
    let out = temp_dir("out");
    let (code, _, stderr) = run(sw(&dist)
        .args(["extract", "--product", "nope", "-o"])
        .arg(&out));
    assert_eq!(code, 1);
    assert!(stderr.contains("product not found: nope"));
}

// --- real distribution smoke tests ------------------------------------

/// The real distribution directory, when the environment provides one.
///
/// Following the `sw-core` golden test rules: unset means skip, but a
/// set-but-invalid path must fail, never silently pass.
fn smoke_dist() -> Option<PathBuf> {
    std::env::var("SW_EXPLORER_TEST_DIST")
        .ok()
        .map(PathBuf::from)
}

#[test]
fn smoke_products_tree_find() {
    let Some(dist) = smoke_dist() else {
        eprintln!("SW_EXPLORER_TEST_DIST not set; skipping smoke test");
        return;
    };
    let (code, stdout, _) = run(sw(&dist).arg("products"));
    assert_eq!(code, 0, "products must succeed on a real distribution");
    let first = stdout
        .lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().next())
        .expect("a real distribution lists at least one product")
        .to_string();
    let (code, _, _) = run(sw(&dist).args(["tree", &first]));
    assert_eq!(code, 0, "tree {first} must succeed");
    let (code, _, _) = run(sw(&dist).args(["find", "lib"]));
    assert_eq!(code, 0, "find lib must succeed");
}
