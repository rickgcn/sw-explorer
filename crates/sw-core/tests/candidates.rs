//! Tests for hardware candidate discovery in `sw-core`
//! ([`Distribution::hardware_candidates`]): every hierarchy level, the
//! expression-tree walk, operator variants, unknown attributes, empty
//! values, deduplication, ordering, and unresolved payloads.
use std::path::{Path, PathBuf};
use sw_core::distribution::Distribution;
use sw_core::mach::candidates::HardwareCandidateSet;

/// A unique temporary directory.
fn temp_root(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "sw-core-candidates-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// Builds a minimal level-9 descriptor for a product with one `sw`
/// image and one `unix` subsystem, with raw attribute blobs per level
/// (tag letter plus payload per attribute).
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
    lp16(&mut bytes, "ALL");
    for _ in 0..7 {
        bytes.extend_from_slice(&0u16.to_be_bytes()); // slots 0-6
    }
    bytes.extend_from_slice(&(unix_attrs.len() as u16).to_be_bytes()); // slot 7
    for attr in unix_attrs {
        lp16(&mut bytes, attr);
    }
    bytes.extend_from_slice(&0u16.to_be_bytes()); // slot 8
    bytes
}

/// Writes the candidate test distribution: one product `cand` carrying
/// MACH expressions on every hierarchy level and on its entries,
/// covering nested `&&`/`||`/`!`, several comparison operators,
/// bare-value shorthand, an unknown attribute, repeated values and an
/// unparseable payload. The subsystem also carries a conditional-flag
/// condition (`IP99`) that is *not* a MACH expression and must never
/// surface as a candidate.
fn write_candidates_dist(root: &Path) {
    std::fs::write(
        root.join("cand"),
        descriptor_bytes(
            "cand",
            &["mCPUBOARD=IP22 && (GFXBOARD=EXPRESS || GFXBOARD=NEWPRESS)"],
            &["mGFXBOARD!=SERVER", "m!(VIDEO=EVO)"],
            &["mMODE>=64bit", "mCPUBOARD<IP30", "DCPUBOARD=IP99"],
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("cand.idb"),
        "f 0755 root sys usr/bin/board26 src/b26 cand.sw.unix sum(1) size(10) cmpsize(0) mach(IP26)\n\
         f 0755 root sys usr/bin/frob src/frob cand.sw.unix sum(1) size(10) cmpsize(0) mach(FROBNICATE=YES)\n\
         f 0755 root sys usr/bin/legacy src/legacy cand.sw.unix sum(1) size(10) cmpsize(0) mach(CPUBOARD=IP22 GFXBOARD=EXPRESS)\n\
         f 0755 root sys usr/bin/garbage src/garbage cand.sw.unix sum(1) size(10) cmpsize(0) mach(=GARBAGE)\n\
         f 0755 root sys usr/bin/again src/again cand.sw.unix sum(1) size(10) cmpsize(0) mach(CPUBOARD=IP22)\n",
    )
    .unwrap();
}

/// The (attribute, values) pairs of a candidate list.
fn candidate_pairs(candidates: &[HardwareCandidateSet]) -> Vec<(String, Vec<String>)> {
    candidates
        .iter()
        .map(|set| (set.attribute.clone(), set.values.clone()))
        .collect()
}

#[test]
fn candidates_cover_every_mach_source_in_first_appearance_order() {
    let root = temp_root("all-sources");
    write_candidates_dist(&root);
    let dist = Distribution::open(&root).unwrap();

    // Product restrictions first, then the image with its subsystems,
    // then the entries in IDB order. Nested && / || / ! trees are
    // walked; every comparison's right-hand side becomes a candidate,
    // whatever the operator; a bare `IP26` is a CPUBOARD comparison;
    // the unknown FROBNICATE attribute is preserved verbatim. IP22 and
    // EXPRESS repeat on later entries and stay deduplicated.
    assert_eq!(
        candidate_pairs(&dist.hardware_candidates()),
        vec![
            (
                "CPUBOARD".to_string(),
                vec!["IP22".to_string(), "IP30".to_string(), "IP26".to_string()],
            ),
            (
                "GFXBOARD".to_string(),
                vec![
                    "EXPRESS".to_string(),
                    "NEWPRESS".to_string(),
                    "SERVER".to_string(),
                ],
            ),
            ("VIDEO".to_string(), vec!["EVO".to_string()]),
            ("MODE".to_string(), vec!["64bit".to_string()]),
            ("FROBNICATE".to_string(), vec!["YES".to_string()]),
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn candidates_ignore_unparseable_payloads() {
    let root = temp_root("unresolved");
    write_candidates_dist(&root);
    let dist = Distribution::open(&root).unwrap();

    // The unparseable entry payload `=GARBAGE` contributes nothing;
    // nothing is guessed from raw text.
    for set in dist.hardware_candidates() {
        assert!(!set.attribute.is_empty());
        for value in &set.values {
            assert!(!value.contains("GARBAGE"));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn candidates_ignore_conditional_flag_conditions() {
    let root = temp_root("flags");
    write_candidates_dist(&root);
    let dist = Distribution::open(&root).unwrap();

    // IP99 only appears in a `default` flag condition, which is not a
    // MACH expression.
    for set in dist.hardware_candidates() {
        assert!(!set.values.iter().any(|value| value == "IP99"));
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn candidates_keep_empty_values() {
    let root = temp_root("empty-value");
    std::fs::write(root.join("gfx"), descriptor_bytes("gfx", &[], &[], &[])).unwrap();
    std::fs::write(
        root.join("gfx.idb"),
        "f 0755 root sys usr/bin/any src/any gfx.sw.unix sum(1) size(10) cmpsize(0)\n\
         f 0755 root sys usr/bin/headless src/headless gfx.sw.unix sum(2) size(10) cmpsize(0) mach(GFXBOARD=)\n",
    )
    .unwrap();
    let dist = Distribution::open(&root).unwrap();

    // `GFXBOARD=` restricts a record to headless boards; the empty
    // right-hand side is a genuine candidate value, never dropped.
    assert_eq!(
        candidate_pairs(&dist.hardware_candidates()),
        vec![("GFXBOARD".to_string(), vec!["".to_string()])]
    );
    let _ = std::fs::remove_dir_all(&root);
}
