//! Tests for the canonical distribution-wide entry identity:
//! [`EntryKey`] uniqueness and round-trips, owning-product identity in
//! search results and selections (including records naming a *foreign*
//! subsystem), planner identity, and diagnostic aggregation.
use std::path::PathBuf;
use sw_core::distribution::{Distribution, EntryKey, LocatedEntry};
use sw_core::extract::ExtractOptions;
use sw_core::idb::EntryId;
use sw_core::mach::eval::HardwareProfile;
use sw_core::plan;
use sw_core::query::Query;

/// A unique temporary directory.
fn temp_root(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "sw-core-identity-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// Builds a minimal level-9 descriptor whose `unix` subsystem carries
/// the given raw attribute blobs (same construction as the other
/// synthetic fixtures: tag letter plus payload per attribute).
fn descriptor_bytes(product: &str, unix_attrs: &[&str]) -> Vec<u8> {
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
    bytes.extend_from_slice(&0u16.to_be_bytes()); // no product attributes
    bytes.extend_from_slice(&1u16.to_be_bytes()); // image count

    bytes.extend_from_slice(&0x0858u16.to_be_bytes()); // image flags
    lp16(&mut bytes, "sw");
    lp16(&mut bytes, "System Software");
    bytes.extend_from_slice(&0u16.to_be_bytes()); // unknown a
    bytes.extend_from_slice(&9999u16.to_be_bytes()); // order candidate
    bytes.extend_from_slice(&0x0102_0305u32.to_be_bytes()); // version
    bytes.extend_from_slice(&0u32.to_be_bytes()); // reserved
    bytes.extend_from_slice(&0u16.to_be_bytes()); // no image attributes
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

/// Builds the identity test distribution:
///
/// ```text
/// alpha  (IDB only) entry 0 names the *foreign* subsystem beta.sw.unix;
///        entries 2-3 share the path usr/share/dup (a fallback conflict)
/// beta   (IDB only) entry 0 is an ordinary IP22-specific record
/// side   (descriptor + IDB) the subsystem MACH blob does not parse
/// ```
///
/// Product order is alphabetical: alpha = 0, beta = 1, side = 2.
fn build_dist() -> PathBuf {
    let root = temp_root("dist");
    std::fs::write(
        root.join("alpha.idb"),
        "f 0755 root sys usr/bin/foreign src/f beta.sw.unix sum(1) size(0) mach(CPUBOARD=IP22)\n\
         f 0644 root sys usr/bin/own src/o alpha.sw.unix sum(2) size(0)\n\
         f 0644 root sys usr/share/dup src/d1 alpha.sw.unix sum(3) size(0)\n\
         f 0644 root sys usr/share/dup src/d2 alpha.sw.unix sum(4) size(0)\n",
    )
    .unwrap();
    std::fs::write(
        root.join("beta.idb"),
        "f 0755 root sys usr/bin/beta src/b beta.sw.unix sum(1) size(0) mach(CPUBOARD=IP22)\n",
    )
    .unwrap();
    std::fs::write(root.join("side"), descriptor_bytes("side", &["m=IP99"])).unwrap();
    std::fs::write(
        root.join("side.idb"),
        "f 0644 root sys bin/side src/s side.sw.unix sum(1) size(0)\n",
    )
    .unwrap();
    root
}

fn open() -> (PathBuf, Distribution) {
    let root = build_dist();
    let dist = Distribution::open(&root).expect("open identity dist");
    (root, dist)
}

/// The product index of `name` in `dist`.
fn product_index(dist: &Distribution, name: &str) -> usize {
    dist.products()
        .iter()
        .position(|p| p.name.as_str() == name)
        .unwrap()
}

#[test]
fn entry_keys_are_unique_across_products() {
    let (_root, dist) = open();
    let alpha = EntryKey {
        product_index: product_index(&dist, "alpha"),
        entry_id: EntryId(0),
    };
    let beta = EntryKey {
        product_index: product_index(&dist, "beta"),
        entry_id: EntryId(0),
    };
    assert_ne!(alpha, beta);
    let set: std::collections::HashSet<_> = [alpha, beta].into_iter().collect();
    assert_eq!(set.len(), 2);
}

#[test]
fn entry_key_roundtrips_through_distribution_entry() {
    let (_root, dist) = open();
    let found = dist.find(&Query::path("*"));
    assert_eq!(found.entries.len(), 6);
    for located in &found.entries {
        let resolved = dist.entry(located.key).expect("key resolves");
        assert_eq!(resolved.key, located.key);
        assert!(std::ptr::eq(resolved.entry, located.entry));
    }
    // Out-of-range keys are rejected, never mis-resolved.
    assert!(
        dist.entry(EntryKey {
            product_index: dist.products().len(),
            entry_id: EntryId(0),
        })
        .is_none()
    );
    assert!(
        dist.entry(EntryKey {
            product_index: 0,
            entry_id: EntryId(9999),
        })
        .is_none()
    );
}

#[test]
fn find_preserves_product_order_idb_order_and_duplicate_paths() {
    let (_root, dist) = open();
    let found = dist.find(&Query::path("*"));
    let keys: Vec<EntryKey> = found.entries.iter().map(|located| located.key).collect();
    assert_eq!(
        keys,
        vec![
            EntryKey {
                product_index: 0,
                entry_id: EntryId(0)
            },
            EntryKey {
                product_index: 0,
                entry_id: EntryId(1)
            },
            EntryKey {
                product_index: 0,
                entry_id: EntryId(2)
            },
            EntryKey {
                product_index: 0,
                entry_id: EntryId(3)
            },
            EntryKey {
                product_index: 1,
                entry_id: EntryId(0)
            },
            EntryKey {
                product_index: 2,
                entry_id: EntryId(0)
            },
        ]
    );
    // Duplicate paths are never deduplicated.
    let dups: Vec<_> = found
        .entries
        .iter()
        .filter(|located| located.entry.path.as_str() == "usr/share/dup")
        .collect();
    assert_eq!(dups.len(), 2);
}

#[test]
fn foreign_subsystem_record_keeps_its_actual_owner() {
    let (_root, dist) = open();
    let found = dist.find(&Query::path_search("foreign").unwrap());
    assert_eq!(found.entries.len(), 1);
    let located = found.entries[0];
    // The record names beta.sw.unix but is owned by alpha.
    assert_eq!(located.entry.subsystem.to_string(), "beta.sw.unix");
    assert_eq!(located.key.product_index, product_index(&dist, "alpha"));
    assert_eq!(located.key.entry_id, EntryId(0));
}

#[test]
fn selection_keys_carry_the_owning_product() {
    let (_root, dist) = open();
    let profile = HardwareProfile::builder().set("CPUBOARD", "IP22").build();
    let selection = dist.select(&profile);

    // The foreign record is selected through its IP22 mach and keeps
    // alpha as its owner; beta's own record keeps beta. The two share
    // EntryId(0) without any identity confusion.
    let foreign = selection
        .selected
        .iter()
        .find(|located| located.entry.path.as_str() == "usr/bin/foreign")
        .expect("foreign record selected");
    assert_eq!(foreign.key.product_index, product_index(&dist, "alpha"));
    let beta = selection
        .selected
        .iter()
        .find(|located| located.entry.path.as_str() == "usr/bin/beta")
        .expect("beta record selected");
    assert_eq!(beta.key.product_index, product_index(&dist, "beta"));
    assert_eq!(foreign.key.entry_id, beta.key.entry_id);
    assert_ne!(foreign.key, beta.key);
}

#[test]
fn conflict_candidates_carry_the_owning_product() {
    let (_root, dist) = open();
    let profile = HardwareProfile::builder().set("CPUBOARD", "IP22").build();
    let selection = dist.select(&profile);

    // Two mach-less fallbacks share usr/share/dup: a conflict whose
    // candidates stay selected, all owned by alpha.
    let conflict = selection
        .conflicts
        .iter()
        .find(|c| c.path.as_str() == "usr/share/dup")
        .expect("dup conflict");
    assert_eq!(conflict.candidates.len(), 2);
    for candidate in &conflict.candidates {
        assert_eq!(candidate.key.product_index, product_index(&dist, "alpha"));
    }

    // side's subsystem MACH is unresolvable: the entry conflicts but is
    // never selected, and its key names side as the owner.
    let unresolved = selection
        .conflicts
        .iter()
        .find(|c| c.path.as_str() == "bin/side")
        .expect("side conflict");
    assert_eq!(unresolved.candidates.len(), 1);
    assert_eq!(
        unresolved.candidates[0].key.product_index,
        product_index(&dist, "side")
    );
    assert!(
        !selection
            .selected
            .iter()
            .any(|located| located.entry.path.as_str() == "bin/side")
    );
}

#[test]
fn planner_membership_uses_actual_owner_identity() {
    let (root, dist) = open();
    let out = root.join("out");
    let foreign = dist.find(&Query::path_search("foreign").unwrap()).entries[0].key;
    let ip22 = HardwareProfile::builder().set("CPUBOARD", "IP22").build();

    // With a matching profile the foreign record is plannable and keeps
    // its alpha identity through the planner.
    let plan = plan::plan_extraction(
        &dist,
        &[foreign],
        &out,
        &ExtractOptions::default(),
        Some(&ip22),
    )
    .expect("the selected foreign record is plannable");
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].key, foreign);
    // The planned entry is exactly the one the key resolves to.
    assert!(std::ptr::eq(
        plan.entries[0].entry,
        dist.entry(foreign).unwrap().entry
    ));

    // With a non-matching profile the same record is excluded: the gate
    // must not mistake beta's same-id entry for it.
    let ip20 = HardwareProfile::builder().set("CPUBOARD", "IP20").build();
    let error = plan::plan_extraction(
        &dist,
        &[foreign],
        &out,
        &ExtractOptions::default(),
        Some(&ip20),
    )
    .expect_err("a non-matching profile excludes the foreign record");
    assert!(
        error
            .to_string()
            .contains("no entries in scope apply to the given hardware profile"),
        "{error}"
    );

    // Checked execution round-trips the identity as well.
    let checked = plan::extract_checked(
        &dist,
        &[foreign],
        &out,
        &ExtractOptions::default(),
        Some(&ip22),
    )
    .expect("checked extraction succeeds");
    assert_eq!(checked.report.extracted, 1);
    assert_eq!(std::fs::read(out.join("usr/bin/foreign")).unwrap(), b"");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn planner_refuses_keys_that_do_not_resolve() {
    let (root, dist) = open();
    let out = root.join("out");
    let bogus_product = EntryKey {
        product_index: dist.products().len(),
        entry_id: EntryId(0),
    };
    let bogus_entry = EntryKey {
        product_index: 0,
        entry_id: EntryId(9999),
    };
    for key in [bogus_product, bogus_entry] {
        let error = plan::plan_extraction(&dist, &[key], &out, &ExtractOptions::default(), None)
            .expect_err("an unresolvable key refuses the plan");
        assert!(error.to_string().contains("does not resolve"), "{error}");
    }
    // Checked execution refuses before writing anything as well.
    let error = plan::extract_checked(
        &dist,
        &[bogus_entry],
        &out,
        &ExtractOptions::default(),
        None,
    )
    .expect_err("checked extraction refuses an unresolvable key");
    assert!(error.to_string().contains("does not resolve"), "{error}");
    assert!(!out.exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn diagnostics_aggregate_distribution_then_products() {
    let root = temp_root("diagnostics");
    // An invalid product name surfaces as a distribution-level error.
    std::fs::write(root.join("bad.name.idb"), "").unwrap();
    // IDB-only products carry one product-level warning each.
    std::fs::write(root.join("x.idb"), "").unwrap();
    std::fs::write(root.join("y.idb"), "").unwrap();
    let dist = Distribution::open(&root).unwrap();

    assert_eq!(dist.diagnostics().len(), 1);
    let all: Vec<_> = dist.all_diagnostics().collect();
    assert_eq!(all.len(), 3);
    assert_eq!(dist.diagnostic_count(), 3);
    // Distribution-level first, then products in product order.
    assert!(all[0].message.contains("bad.name"));
    assert_eq!(all[1].origin.as_deref(), root.join("x.idb").to_str());
    assert_eq!(all[2].origin.as_deref(), root.join("y.idb").to_str());
    let _ = std::fs::remove_dir_all(&root);
}

/// Every entry of the distribution, to guard the fixtures themselves.
#[test]
fn every_entry_resolves_by_key() {
    let (_root, dist) = open();
    for (product_index, product) in dist.products().iter().enumerate() {
        for entry in &product.entries {
            let key = EntryKey {
                product_index,
                entry_id: entry.id,
            };
            let located: LocatedEntry = dist.entry(key).expect("every entry resolves");
            assert!(std::ptr::eq(located.entry, entry));
        }
    }
}
