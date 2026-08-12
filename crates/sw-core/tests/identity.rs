//! Tests for the canonical distribution-wide entry identity:
//! [`EntryKey`] uniqueness and round-trips, owning-product identity in
//! search results and selections (including records naming a *foreign*
//! subsystem), planner identity, and diagnostic aggregation.
use std::path::PathBuf;
use sw_core::distribution::{Distribution, EntryKey, ImageKey, LocatedEntry, SubsystemKey};
use sw_core::error::Error;
use sw_core::extract::{self, ExtractOptions};
use sw_core::idb::EntryId;
use sw_core::image::PayloadResolution;
use sw_core::mach::eval::HardwareProfile;
use sw_core::names::{ImageName, SubsystemName};
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

/// Builds a malformed-but-preserved descriptor for `product`: two
/// image records both named `sw` — the first carrying two subsystem
/// records both named `unix`, the second one — so one product holds
/// several logical images and subsystems with identical qualified
/// names.
fn descriptor_bytes_dup(product: &str) -> Vec<u8> {
    fn lp16(bytes: &mut Vec<u8>, s: &str) {
        bytes.extend_from_slice(&(s.len() as u16).to_be_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }
    fn push_image_record(bytes: &mut Vec<u8>, unix_records: usize) {
        bytes.extend_from_slice(&0x0858u16.to_be_bytes()); // image flags
        lp16(bytes, "sw");
        lp16(bytes, "System Software");
        bytes.extend_from_slice(&0u16.to_be_bytes()); // unknown a
        bytes.extend_from_slice(&9999u16.to_be_bytes()); // order candidate
        bytes.extend_from_slice(&0x0102_0305u32.to_be_bytes()); // version
        bytes.extend_from_slice(&0u32.to_be_bytes()); // reserved
        bytes.extend_from_slice(&0u16.to_be_bytes()); // no image attributes
        bytes.extend_from_slice(&(unix_records as u16).to_be_bytes());
        for _ in 0..unix_records {
            bytes.extend_from_slice(&0x0852u16.to_be_bytes()); // subsystem flags
            lp16(bytes, "unix");
            lp16(bytes, "UNIX Kernel");
            lp16(bytes, "ALL");
            for _ in 0..9 {
                bytes.extend_from_slice(&0u16.to_be_bytes()); // slots 0-8
            }
        }
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
    bytes.extend_from_slice(&2u16.to_be_bytes()); // image count
    push_image_record(&mut bytes, 2); // `sw`, two `unix` records
    push_image_record(&mut bytes, 1); // `sw` again, one `unix` record
    bytes
}

/// Builds the duplicate-name distribution: product `dup` ships the
/// duplicate-image descriptor above plus an IDB whose two payload
/// records name `dup.sw.unix` (attachment assigns them to the first
/// matching logical subsystem) and the matching `dup.sw` archive.
///
/// Logical tree:
///
/// ```text
/// dup
/// ├── dup.sw #0
/// │   ├── dup.sw.unix #0   ← both IDB entries attach here
/// │   └── dup.sw.unix #1   (empty)
/// └── dup.sw #1
///     └── dup.sw.unix      (empty)
/// ```
fn build_dup_dist() -> PathBuf {
    let root = temp_root("dup");
    std::fs::write(root.join("dup"), descriptor_bytes_dup("dup")).unwrap();
    std::fs::write(
        root.join("dup.idb"),
        "f 0644 root sys dup/a.txt src/a dup.sw.unix sum(1) size(4) cmpsize(0)\n\
         f 0644 root sys dup/b.txt src/b dup.sw.unix sum(2) size(4) cmpsize(0)\n",
    )
    .unwrap();
    let mut archive = b"im001V999P00\0".to_vec();
    for (name, payload) in [("dup/a.txt", b"AAAA"), ("dup/b.txt", b"BBBB")] {
        archive.extend_from_slice(&(name.len() as u16).to_be_bytes());
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(payload);
    }
    std::fs::write(root.join("dup.sw"), archive).unwrap();
    root
}

/// Builds the foreign-archive distribution: product `alpha` (IDB only)
/// carries two payload records naming the foreign subsystem
/// `beta.sw.unix`; the physical `beta.sw` archive holds their payloads,
/// shifted by one leading unreferenced record. There is no product
/// `beta` anywhere in the distribution.
fn build_foreign_archive_dist() -> PathBuf {
    let root = temp_root("foreign-archive");
    std::fs::write(
        root.join("alpha.idb"),
        "f 0644 root sys first.txt src/f beta.sw.unix sum(1) size(4) cmpsize(0)\n\
         f 0644 root sys second.txt src/s beta.sw.unix sum(2) size(4) cmpsize(0)\n",
    )
    .unwrap();
    let mut archive = b"im001V999P00\0".to_vec();
    for (name, payload) in [
        ("zz.pad", b"PADPAD".as_slice()),
        ("first.txt", b"FI!!".as_slice()),
        ("second.txt", b"SE!!".as_slice()),
    ] {
        archive.extend_from_slice(&(name.len() as u16).to_be_bytes());
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(payload);
    }
    std::fs::write(root.join("beta.sw"), archive).unwrap();
    root
}

/// Builds the interleaved distribution: one IDB-only product `mix`
/// whose two subsystems alternate in the IDB (`a/one`, `b/one`,
/// `a/two`, `b/two`), so an image scope must not regroup records by
/// subsystem.
fn build_interleaved_dist() -> PathBuf {
    let root = temp_root("interleaved");
    std::fs::write(
        root.join("mix.idb"),
        "f 0644 root sys a/one src/a1 mix.sw.aaa sum(1) size(1)\n\
         f 0644 root sys b/one src/b1 mix.sw.bbb sum(2) size(1)\n\
         f 0644 root sys a/two src/a2 mix.sw.aaa sum(3) size(1)\n\
         f 0644 root sys b/two src/b2 mix.sw.bbb sum(4) size(1)\n",
    )
    .unwrap();
    root
}

/// Builds a descriptor-only product: one image, one subsystem, no IDB,
/// hence no entries at all.
fn build_bare_dist() -> PathBuf {
    let root = temp_root("bare");
    std::fs::write(root.join("bare"), descriptor_bytes("bare", &[])).unwrap();
    root
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

// ---------------------------------------------------------------------
// Hierarchy identity: ImageKey / SubsystemKey, name lookups and exact
// scopes. A qualified name is what the media called an object; a key
// is where the object lives.
// ---------------------------------------------------------------------

#[test]
fn image_and_subsystem_keys_are_unique_across_products() {
    let (_root, dist) = open();
    let alpha = product_index(&dist, "alpha");
    let beta = product_index(&dist, "beta");
    let alpha_image = ImageKey {
        product_index: alpha,
        image_index: 0,
    };
    let beta_image = ImageKey {
        product_index: beta,
        image_index: 0,
    };
    assert_ne!(alpha_image, beta_image);
    let alpha_subsystem = SubsystemKey {
        product_index: alpha,
        image_index: 0,
        subsystem_index: 0,
    };
    let beta_subsystem = SubsystemKey {
        product_index: beta,
        image_index: 0,
        subsystem_index: 0,
    };
    assert_ne!(alpha_subsystem, beta_subsystem);
    let images: std::collections::HashSet<_> = [alpha_image, beta_image].into_iter().collect();
    assert_eq!(images.len(), 2);
    let subsystems: std::collections::HashSet<_> =
        [alpha_subsystem, beta_subsystem].into_iter().collect();
    assert_eq!(subsystems.len(), 2);
}

#[test]
fn image_and_subsystem_keys_roundtrip_and_bounds_check() {
    let (_root, dist) = open();
    for (product_index, product) in dist.products().iter().enumerate() {
        for (image_index, image) in product.images.iter().enumerate() {
            let key = ImageKey {
                product_index,
                image_index,
            };
            let located = dist.image(key).expect("image key resolves");
            assert_eq!(located.key, key);
            assert!(std::ptr::eq(located.image, image));
            for (subsystem_index, subsystem) in image.subsystems.iter().enumerate() {
                let key = SubsystemKey {
                    product_index,
                    image_index,
                    subsystem_index,
                };
                let located = dist.subsystem(key).expect("subsystem key resolves");
                assert_eq!(located.key, key);
                assert!(std::ptr::eq(located.subsystem, subsystem));
            }
        }
    }
    // Out-of-range indices at every level are rejected, never
    // mis-resolved.
    let products = dist.products().len();
    assert!(
        dist.image(ImageKey {
            product_index: products,
            image_index: 0,
        })
        .is_none()
    );
    assert!(
        dist.image(ImageKey {
            product_index: 0,
            image_index: 9999,
        })
        .is_none()
    );
    assert!(
        dist.subsystem(SubsystemKey {
            product_index: products,
            image_index: 0,
            subsystem_index: 0,
        })
        .is_none()
    );
    assert!(
        dist.subsystem(SubsystemKey {
            product_index: 0,
            image_index: 9999,
            subsystem_index: 0,
        })
        .is_none()
    );
    assert!(
        dist.subsystem(SubsystemKey {
            product_index: 0,
            image_index: 0,
            subsystem_index: 9999,
        })
        .is_none()
    );
}

#[test]
fn name_lookups_return_every_same_name_object_in_order() {
    let (_root, dist) = open();
    let alpha = product_index(&dist, "alpha");
    let beta = product_index(&dist, "beta");

    // alpha's IDB references the foreign beta.sw.unix, so both
    // products carry a logical beta.sw image / beta.sw.unix subsystem.
    // The lookup reports every candidate in distribution order; no
    // first match wins, nothing is deduplicated.
    let image_name = ImageName::parse("beta.sw").unwrap();
    let images = dist.images_named(&image_name);
    let keys: Vec<ImageKey> = images.iter().map(|located| located.key).collect();
    assert_eq!(
        keys,
        vec![
            ImageKey {
                product_index: alpha,
                image_index: 0,
            },
            ImageKey {
                product_index: beta,
                image_index: 0,
            },
        ]
    );
    assert!(
        images
            .iter()
            .all(|located| located.image.name == image_name)
    );

    let subsystem_name = SubsystemName::parse("beta.sw.unix").unwrap();
    let subsystems = dist.subsystems_named(&subsystem_name);
    let keys: Vec<SubsystemKey> = subsystems.iter().map(|located| located.key).collect();
    assert_eq!(
        keys,
        vec![
            SubsystemKey {
                product_index: alpha,
                image_index: 0,
                subsystem_index: 0,
            },
            SubsystemKey {
                product_index: beta,
                image_index: 0,
                subsystem_index: 0,
            },
        ]
    );

    // A name nothing carries matches nothing; a unique name matches
    // exactly once.
    assert!(
        dist.images_named(&ImageName::parse("nope.sw").unwrap())
            .is_empty()
    );
    assert_eq!(
        dist.images_named(&ImageName::parse("side.sw").unwrap())
            .len(),
        1
    );
    assert!(
        dist.subsystems_named(&SubsystemName::parse("alpha.sw.unix").unwrap())
            .len()
            == 1
    );
}

#[test]
fn qualified_name_product_segment_is_not_the_containing_product() {
    let (_root, dist) = open();
    let alpha = product_index(&dist, "alpha");
    let images = dist.images_named(&ImageName::parse("beta.sw").unwrap());
    // The container of the first candidate is alpha; the qualified
    // media name claims beta. Both facts coexist — this is a legal
    // state, not an inconsistency to be repaired.
    assert_eq!(images[0].key.product_index, alpha);
    assert_eq!(images[0].image.name.product().as_str(), "beta");
}

#[test]
fn exact_scopes_use_attachment_not_qualified_names() {
    let (_root, dist) = open();
    let alpha = product_index(&dist, "alpha");
    let beta = product_index(&dist, "beta");
    let images = dist.images_named(&ImageName::parse("beta.sw").unwrap());

    // Two logical images share the name beta.sw; their exact entry
    // scopes are disjoint and each holds only its container's records.
    let alpha_entries = dist
        .entries_in_image(images[0].key)
        .expect("alpha image scope");
    let beta_entries = dist
        .entries_in_image(images[1].key)
        .expect("beta image scope");
    assert_eq!(alpha_entries.len(), 1);
    assert_eq!(alpha_entries[0].entry.path.as_str(), "usr/bin/foreign");
    assert!(
        alpha_entries
            .iter()
            .all(|located| located.key.product_index == alpha)
    );
    assert_eq!(beta_entries.len(), 1);
    assert_eq!(beta_entries[0].entry.path.as_str(), "usr/bin/beta");
    assert!(
        beta_entries
            .iter()
            .all(|located| located.key.product_index == beta)
    );

    // Same for the two same-named subsystems.
    let subsystems = dist.subsystems_named(&SubsystemName::parse("beta.sw.unix").unwrap());
    let alpha_entries = dist
        .entries_in_subsystem(subsystems[0].key)
        .expect("alpha subsystem scope");
    let beta_entries = dist
        .entries_in_subsystem(subsystems[1].key)
        .expect("beta subsystem scope");
    assert_eq!(alpha_entries.len(), 1);
    assert_eq!(alpha_entries[0].entry.path.as_str(), "usr/bin/foreign");
    assert_eq!(beta_entries.len(), 1);
    assert_eq!(beta_entries[0].entry.path.as_str(), "usr/bin/beta");
}

#[test]
fn entries_in_image_keeps_interleaved_idb_order() {
    let root = build_interleaved_dist();
    let dist = Distribution::open(&root).unwrap();
    let mix = product_index(&dist, "mix");
    let entries = dist
        .entries_in_image(ImageKey {
            product_index: mix,
            image_index: 0,
        })
        .expect("image scope resolves");
    let paths: Vec<&str> = entries
        .iter()
        .map(|located| located.entry.path.as_str())
        .collect();
    // A1 B1 A2 B2: exact IDB order, never regrouped by subsystem.
    assert_eq!(paths, vec!["a/one", "b/one", "a/two", "b/two"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn descriptor_only_scopes_are_empty_not_unresolvable() {
    let root = build_bare_dist();
    let dist = Distribution::open(&root).unwrap();
    let image_key = ImageKey {
        product_index: 0,
        image_index: 0,
    };
    let subsystem_key = SubsystemKey {
        product_index: 0,
        image_index: 0,
        subsystem_index: 0,
    };
    // A valid image or subsystem without attached entries is an empty
    // scope, not a resolution failure.
    assert!(
        dist.entries_in_image(image_key)
            .expect("valid image scope")
            .is_empty()
    );
    assert!(
        dist.entries_in_subsystem(subsystem_key)
            .expect("valid subsystem scope")
            .is_empty()
    );
    // `None` only means the key does not resolve.
    assert!(
        dist.entries_in_image(ImageKey {
            product_index: 0,
            image_index: 1,
        })
        .is_none()
    );
    assert!(
        dist.entries_in_subsystem(SubsystemKey {
            product_index: 0,
            image_index: 0,
            subsystem_index: 1,
        })
        .is_none()
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn same_owner_duplicate_names_have_disjoint_exact_scopes() {
    let root = build_dup_dist();
    let dist = Distribution::open(&root).unwrap();
    let dup = product_index(&dist, "dup");

    // The name lookups see every logical object carrying the name, in
    // hierarchy order: two images, three subsystems.
    let images = dist.images_named(&ImageName::parse("dup.sw").unwrap());
    let image_keys: Vec<ImageKey> = images.iter().map(|located| located.key).collect();
    assert_eq!(
        image_keys,
        vec![
            ImageKey {
                product_index: dup,
                image_index: 0,
            },
            ImageKey {
                product_index: dup,
                image_index: 1,
            },
        ]
    );
    let subsystems = dist.subsystems_named(&SubsystemName::parse("dup.sw.unix").unwrap());
    let subsystem_keys: Vec<SubsystemKey> = subsystems.iter().map(|located| located.key).collect();
    assert_eq!(
        subsystem_keys,
        vec![
            SubsystemKey {
                product_index: dup,
                image_index: 0,
                subsystem_index: 0,
            },
            SubsystemKey {
                product_index: dup,
                image_index: 0,
                subsystem_index: 1,
            },
            SubsystemKey {
                product_index: dup,
                image_index: 1,
                subsystem_index: 0,
            },
        ]
    );

    // Attachment assigns both IDB entries to the first matching
    // subsystem; the same-named objects keep their own, disjoint
    // scopes — the shared name never merges them.
    let first = dist
        .entries_in_image(image_keys[0])
        .expect("image #0 scope");
    let second = dist
        .entries_in_image(image_keys[1])
        .expect("image #1 scope");
    assert_eq!(first.len(), 2);
    assert!(second.is_empty());
    let attached = dist
        .entries_in_subsystem(subsystem_keys[0])
        .expect("subsystem #0 scope");
    assert_eq!(attached.len(), 2);
    for key in &subsystem_keys[1..] {
        assert!(
            dist.entries_in_subsystem(*key)
                .expect("scope resolves")
                .is_empty()
        );
    }

    // Layouts follow attachment as well: only the image the entries
    // were attached to learns their payload layout, even though the
    // other image carries the same name (and hence the same archive
    // path).
    let image0 = dist.image(image_keys[0]).unwrap().image;
    let image1 = dist.image(image_keys[1]).unwrap().image;
    assert_eq!(image0.archive.layout.payloads.len(), 2);
    assert!(image1.archive.layout.payloads.is_empty());

    // And the payloads read back through the attached image's archive.
    let mut reader = dist.image_reader();
    let payload = reader.read(attached[0].key).expect("read dup/a.txt");
    assert_eq!(payload.bytes, b"AAAA");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn reader_reads_foreign_payloads_without_a_matching_product() {
    let root = build_foreign_archive_dist();
    let dist = Distribution::open(&root).unwrap();
    // The premise: no product `beta` exists anywhere in the
    // distribution; only the physical beta.sw archive file does.
    assert_eq!(dist.products().len(), 1);
    assert!(dist.product("beta").is_none());

    let alpha = product_index(&dist, "alpha");
    let first = EntryKey {
        product_index: alpha,
        entry_id: EntryId(0),
    };
    let second = EntryKey {
        product_index: alpha,
        entry_id: EntryId(1),
    };
    let mut reader = dist.image_reader();

    // The archive carries one leading record no entry references, so
    // the first read resynchronizes (cross-checked against the next
    // payload record of the *owning* product) and the second read hits
    // through the learned delta.
    let payload = reader
        .read(first)
        .expect("read first.txt from the foreign archive");
    assert_eq!(payload.bytes, b"FI!!");
    assert_eq!(
        payload.location.resolution,
        PayloadResolution::Resynced { delta: Some(14) }
    );
    let payload = reader
        .read(second)
        .expect("read second.txt through the learned delta");
    assert_eq!(payload.bytes, b"SE!!");
    assert_eq!(
        payload.location.resolution,
        PayloadResolution::Delta { delta: 14 }
    );

    // Checked extraction of the foreign record succeeds as well: plan,
    // write and payload read all resolve through the owning product.
    let out = root.join("out");
    let checked = plan::extract_checked(&dist, &[first], &out, &ExtractOptions::default(), None)
        .expect("checked extraction of a foreign record succeeds");
    assert_eq!(checked.report.extracted, 1);
    assert!(checked.report.failures.is_empty());
    assert_eq!(std::fs::read(out.join("first.txt")).unwrap(), b"FI!!");
    let _ = std::fs::remove_dir_all(&root);
}

/// The payload reader and the low-level writer resolve every key
/// against the distribution they are themselves bound to: entry
/// metadata can never be paired with another distribution's hierarchy
/// identity, and an out-of-range key fails honestly instead of
/// panicking.
#[test]
fn reader_and_writer_resolve_keys_against_their_own_distribution() {
    let (root, dist) = open();
    let mut reader = dist.image_reader();

    // Numerically valid keys identify the entry at those indices in
    // *this* distribution (the session-local key contract): (0, 0) is
    // alpha's payload-less `usr/bin/foreign` here, whatever another
    // distribution may keep at the same numeric key.
    let error = reader
        .read(EntryKey {
            product_index: 0,
            entry_id: EntryId(0),
        })
        .unwrap_err();
    assert!(
        matches!(error, Error::PayloadNotFound { .. }),
        "expected this distribution's entry metadata, got {error}"
    );
    assert!(error.to_string().contains("usr/bin/foreign"));

    // Out of range in either half: an honest EntryNotFound, no panic.
    let bogus_product = EntryKey {
        product_index: 999,
        entry_id: EntryId(0),
    };
    assert!(matches!(
        reader.read(bogus_product),
        Err(Error::EntryNotFound { .. })
    ));
    let bogus_entry = EntryKey {
        product_index: 0,
        entry_id: EntryId(999),
    };
    assert!(matches!(
        reader.read(bogus_entry),
        Err(Error::EntryNotFound { .. })
    ));

    // The low-level writer reports an unresolvable key as a failure and
    // writes nothing.
    let out = root.join("out");
    let report = extract::extract_unchecked(
        &mut reader,
        &[bogus_product],
        &out,
        &ExtractOptions::default(),
    );
    assert_eq!(report.extracted, 0);
    assert_eq!(report.failures.len(), 1);
    assert!(!out.exists());

    let _ = std::fs::remove_dir_all(&root);
}

/// The attachment invariant every scope and the payload reader rely
/// on: every entry belongs to exactly one logical subsystem.
#[test]
fn every_entry_is_attached_to_exactly_one_subsystem() {
    for root in [
        build_dist(),
        build_dup_dist(),
        build_foreign_archive_dist(),
        build_interleaved_dist(),
    ] {
        let dist = Distribution::open(&root).unwrap();
        for product in dist.products() {
            let mut attached: Vec<usize> = product
                .images
                .iter()
                .flat_map(|image| image.subsystems.iter())
                .flat_map(|subsystem| subsystem.entry_ids.iter().map(|id| id.0))
                .collect();
            attached.sort_unstable();
            let expected: Vec<usize> = product.entries.iter().map(|entry| entry.id.0).collect();
            assert_eq!(attached, expected, "product {}", product.name);
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
