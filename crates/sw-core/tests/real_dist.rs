//! Golden tests against real IRIX distribution directories.
//!
//! These tests only run when `SW_EXPLORER_TEST_DIST` points at a real
//! `dist` directory, e.g.:
//!
//! ```sh
//! SW_EXPLORER_TEST_DIST="/path/to/IRIX 5.3/dist" cargo test -p sw-core --test real_dist
//! ```
use std::path::PathBuf;
use sw_core::diagnostic::Severity;
use sw_core::distribution::Distribution;
use sw_core::image::PayloadResolution;
use sw_core::mach::eval::HardwareProfile;
use sw_core::query::Query;

fn dist_path() -> Option<PathBuf> {
    // Unset means skip; set-but-invalid must fail loudly, otherwise a
    // typo (or a wrong working directory) silently skips every golden
    // test and reports green.
    let raw = std::env::var_os("SW_EXPLORER_TEST_DIST")?;
    let path = PathBuf::from(raw);
    assert!(
        path.is_dir(),
        "SW_EXPLORER_TEST_DIST is set but not a directory: {}",
        path.display()
    );
    Some(path)
}

/// Every payload-bearing entry of every product must be readable, and on
/// intact media the layout algorithm must hit the exact record offset.
#[test]
fn all_payloads_read_exact() {
    let Some(path) = dist_path() else {
        eprintln!("SW_EXPLORER_TEST_DIST not set; skipping");
        return;
    };
    let dist = Distribution::open(&path).expect("open distribution");
    assert!(!dist.products().is_empty());
    let mut reader = dist.image_reader();
    let mut total = 0usize;
    let mut exact = 0usize;
    let mut recovered = Vec::new();
    let mut failed = Vec::new();

    for product in dist.products() {
        for entry in &product.entries {
            if entry.payload.is_none() {
                continue;
            }
            total += 1;
            match reader.read(entry) {
                Ok(payload) => match payload.location.resolution {
                    PayloadResolution::Exact => exact += 1,
                    other => recovered.push((entry.path.to_string(), other)),
                },
                Err(error) => failed.push((entry.path.to_string(), error.to_string())),
            }
        }
    }

    eprintln!(
        "payloads: {total} total, {exact} exact, {} recovered",
        recovered.len()
    );
    for (path, resolution) in &recovered {
        eprintln!("  recovered: {path} via {resolution:?}");
    }
    for (path, error) in &failed[..failed.len().min(20)] {
        eprintln!("  failed: {path}: {error}");
    }
    assert!(
        failed.is_empty(),
        "{} payloads failed to read",
        failed.len()
    );
    assert_eq!(exact, total, "non-exact resolutions: {recovered:?}");
}

/// Decompressed payload sizes must equal the IDB `size(...)` attribute.
#[test]
fn decoded_sizes_match_idb() {
    let Some(path) = dist_path() else {
        eprintln!("SW_EXPLORER_TEST_DIST not set; skipping");
        return;
    };
    let dist = Distribution::open(&path).expect("open distribution");
    let mut reader = dist.image_reader();
    let mut checked = 0usize;

    for product in dist.products() {
        for entry in &product.entries {
            let (Some(_), Some(expected_size)) = (&entry.payload, entry.size()) else {
                continue;
            };
            let payload = reader.read(entry).expect("read payload");
            let decoded = payload.decode().expect("decode payload");
            assert_eq!(
                decoded.len() as u64,
                expected_size,
                "size mismatch for {}",
                entry.path
            );
            checked += 1;
        }
    }
    assert!(checked > 0);
    eprintln!("decoded sizes verified for {checked} entries");
}

/// Query and selection must work end to end on real data.
#[test]
fn query_and_selection_run() {
    let Some(path) = dist_path() else {
        eprintln!("SW_EXPLORER_TEST_DIST not set; skipping");
        return;
    };
    let dist = Distribution::open(&path).expect("open distribution");

    let everything = dist.find(&Query::path("*"));
    let total: usize = dist.products().iter().map(|p| p.entries.len()).sum();
    assert_eq!(everything.entries.len(), total);

    let profile = HardwareProfile::builder()
        .set("CPUBOARD", "IP22")
        .set("CPUARCH", "R4400")
        .set("GFXBOARD", "EXPRESS")
        .set("MODE", "32bit")
        .build();
    let selection = dist.select(&profile);
    assert!(!selection.selected.is_empty());
    eprintln!(
        "selected {} entries, {} conflicts",
        selection.selected.len(),
        selection.conflicts.len()
    );
}

/// Closure invariant: every image archive must be covered from byte 13 to
/// exactly its end by the records the IDB declares. This catches payload
/// metadata the parser failed to recognize (which the payload-reading
/// test above cannot see, since it only visits located payloads).
#[test]
fn image_archives_walk_exactly_to_eof() {
    let Some(path) = dist_path() else {
        eprintln!("SW_EXPLORER_TEST_DIST not set; skipping");
        return;
    };
    let dist = Distribution::open(&path).expect("open distribution");

    let mut checked = 0usize;
    for product in dist.products() {
        for image in &product.images {
            // The last payload-bearing entry of this image, in IDB order.
            let Some(last) = product
                .entries
                .iter()
                .rfind(|e| e.payload.as_ref().is_some_and(|l| l.image == image.name))
            else {
                continue;
            };
            let locator = last.payload.as_ref().unwrap();
            // Latin-1 raw name: one archive byte per character.
            let name_len = last.raw_path.chars().count() as u64;
            if let (Some(expected), Some(encoded_size)) =
                (locator.expected_record_offset, locator.encoded_size)
            {
                let end = expected + 2 + name_len + encoded_size;
                assert_eq!(
                    end, image.archive.file_size,
                    "image {} does not walk exactly to EOF (last entry {})",
                    image.name, last.path
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0);
    eprintln!("{checked} image archives walk exactly to EOF");
}

/// No valid IDB line may be skipped: real media must produce zero
/// error-severity diagnostics.
#[test]
fn no_error_diagnostics_on_real_media() {
    let Some(path) = dist_path() else {
        eprintln!("SW_EXPLORER_TEST_DIST not set; skipping");
        return;
    };
    let dist = Distribution::open(&path).expect("open distribution");

    let mut errors = Vec::new();
    for diagnostic in dist.all_diagnostics() {
        if diagnostic.severity == Severity::Error {
            match &diagnostic.origin {
                Some(origin) => errors.push(format!("{origin}: {}", diagnostic.message)),
                None => errors.push(diagnostic.message.clone()),
            }
        }
    }
    for error in errors.iter().take(20) {
        eprintln!("  {error}");
    }
    assert!(errors.is_empty(), "{} error diagnostics", errors.len());
}

/// The descriptor is the hierarchy authority: every product on real
/// media must parse byte-exactly, and its declared tree must reconcile
/// with the IDB. The single known corpus anomaly is IRIX 6.3 declaring
/// `eoe.sw.cdsio` ("Multiport Serial Board Support") in the descriptor
/// without shipping any of its entries in the IDB; anything else is a
/// regression.
#[test]
fn descriptors_parse_and_reconcile_with_idb() {
    let Some(path) = dist_path() else {
        eprintln!("SW_EXPLORER_TEST_DIST not set; skipping");
        return;
    };
    let dist = Distribution::open(&path).expect("open distribution");
    assert!(!dist.products().is_empty());

    let mut descriptor_only = Vec::new();
    let mut idb_only = Vec::new();
    let mut subsystems = 0usize;
    for product in dist.products() {
        let descriptor = product
            .descriptor
            .as_ref()
            .unwrap_or_else(|| panic!("{}: descriptor did not parse", product.name));
        assert_eq!(descriptor.name, product.name.as_str());
        assert_eq!(descriptor.images.len(), product.images.len());
        for image in &product.images {
            for subsystem in &image.subsystems {
                subsystems += 1;
                match (subsystem.presence.descriptor, subsystem.presence.idb) {
                    (true, false) => descriptor_only.push(subsystem.name.to_string()),
                    (false, true) => idb_only.push(subsystem.name.to_string()),
                    _ => {}
                }
            }
        }
    }
    assert!(subsystems > 0);
    eprintln!(
        "{subsystems} subsystems reconciled; descriptor-only: {descriptor_only:?}, \
         idb-only: {idb_only:?}"
    );
    assert!(
        descriptor_only.is_empty() || descriptor_only == ["eoe.sw.cdsio".to_string()],
        "unexpected descriptor-only subsystems: {descriptor_only:?}"
    );
    assert!(idb_only.is_empty(), "IDB-only subsystems: {idb_only:?}");
}

/// Descriptor semantics must be fully decoded on real media: every
/// descriptor-backed subsystem carries decoded flags, mach, autominiroot
/// and rules (IDB-only subsystems carry nothing), and the decoded values
/// must respect the writer's invariants.
#[test]
fn descriptor_semantics_are_decoded() {
    let Some(path) = dist_path() else {
        eprintln!("SW_EXPLORER_TEST_DIST not set; skipping");
        return;
    };
    let dist = Distribution::open(&path).expect("open distribution");

    let mut patches = 0usize;
    let mut follows_records = 0usize;
    let mut conditional_flags = 0usize;
    let mut mach_restricted = 0usize;
    let mut miniroot = 0usize;
    for product in dist.products() {
        if product.descriptor.is_some() {
            let mach = product.mach.as_ref().unwrap_or_else(|| {
                panic!("{}: descriptor product without decoded mach", product.name)
            });
            assert!(
                mach.unresolved.is_empty(),
                "{}: unparsable product mach expressions: {:?}",
                product.name,
                mach.unresolved
            );
        }
        for image in &product.images {
            if let Some(mach) = &image.mach {
                assert!(
                    mach.unresolved.is_empty(),
                    "{}: unparsable image mach expressions: {:?}",
                    image.name,
                    mach.unresolved
                );
            }
            for subsystem in &image.subsystems {
                match (
                    subsystem.presence.descriptor,
                    &subsystem.flags,
                    &subsystem.rules,
                ) {
                    (true, Some(flags), Some(rules)) => {
                        let mach = subsystem.mach.as_ref().unwrap();
                        assert!(
                            mach.unresolved.is_empty(),
                            "{}: unparsable subsystem mach expressions: {:?}",
                            subsystem.name,
                            mach.unresolved
                        );
                        assert!(subsystem.autominiroot.is_some());
                        // `follows` is only ever encoded for patches.
                        if !rules.follows.is_empty() {
                            follows_records += rules.follows.len();
                            assert!(
                                flags.patch,
                                "{}: follows records on a non-patch subsystem",
                                subsystem.name
                            );
                        }
                        // The miniroot decode is exactly "inplace cleared".
                        assert_eq!(
                            flags.miniroot != sw_core::descriptor::model::ConditionalFlag::No,
                            !flags.inplace,
                            "{}: miniroot/inplace mismatch",
                            subsystem.name
                        );
                        if flags.patch {
                            patches += 1;
                        }
                        if matches!(
                            flags.required,
                            sw_core::descriptor::model::ConditionalFlag::When(_)
                        ) || matches!(
                            flags.default,
                            sw_core::descriptor::model::ConditionalFlag::When(_)
                        ) {
                            conditional_flags += 1;
                        }
                        if !subsystem.mach.as_ref().unwrap().is_empty() {
                            mach_restricted += 1;
                        }
                        if flags.miniroot != sw_core::descriptor::model::ConditionalFlag::No {
                            miniroot += 1;
                        }
                    }
                    (false, None, None) => {
                        assert!(subsystem.mach.is_none());
                        assert!(subsystem.autominiroot.is_none());
                    }
                    _ => panic!("{}: presence/decoded semantics mismatch", subsystem.name),
                }
            }
        }
    }
    eprintln!(
        "semantics: {patches} patch subsystems, {follows_records} follows records, \
         {conditional_flags} hardware-conditional flags, {mach_restricted} mach-restricted, \
         {miniroot} miniroot"
    );
}
