//! Command implementations: call `sw-core`, gather results, render them.
//!
//! No IRIX format logic lives here; every question is answered through
//! the public `sw-core` API.

use crate::cli::{Cli, Command, ExtractArgs, FindArgs, SelectArgs, ShowArgs, TreeArgs};
use crate::output;
use anyhow::{Context, Result, anyhow, bail};
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use sw_core::descriptor::model::Subsystem;
use sw_core::diagnostic::{Diagnostic, Severity};
use sw_core::distribution::{Distribution, Product};
use sw_core::error::Error as CoreError;
use sw_core::extract::{self, DecodeMode, ExtractOptions, PathMode};
use sw_core::idb::{Entry, FileType};
use sw_core::image::Image;
use sw_core::mach::eval::HardwareProfile;
use sw_core::query::Query;
use sw_core::selection::SelectionConflict;

/// Executes one parsed command line.
pub fn run(cli: Cli) -> Result<()> {
    let dist = Distribution::open(&cli.dist)
        .with_context(|| format!("cannot open distribution {}", cli.dist.display()))?;
    report_diagnostics(&dist);
    match &cli.command {
        Command::Products => products(&dist),
        Command::Tree(args) => tree(&dist, args),
        Command::Show(args) => show(&dist, args),
        Command::Find(args) => find(&dist, args),
        Command::Select(args) => select(&dist, args),
        Command::Extract(args) => extract_command(&dist, args),
    }
}

/// Prints non-fatal problems found while opening the distribution.
fn report_diagnostics(dist: &Distribution) {
    let print = |diagnostic: &Diagnostic| {
        let severity = match diagnostic.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        match &diagnostic.origin {
            Some(origin) => eprintln!("{severity}: {} ({origin})", diagnostic.message),
            None => eprintln!("{severity}: {}", diagnostic.message),
        }
    };
    for diagnostic in dist.diagnostics() {
        print(diagnostic);
    }
    for product in dist.products() {
        for diagnostic in &product.diagnostics {
            print(diagnostic);
        }
    }
}

/// Builds a hardware profile from `--mach` values.
///
/// A bare value is the documented `CPUBOARD` shorthand; the profile
/// builder supports several values per attribute. Values were already
/// validated by the argument parser.
fn parse_mach(mach: &[String]) -> Result<(Vec<(String, String)>, HardwareProfile)> {
    let mut pairs = Vec::new();
    let mut builder = HardwareProfile::builder();
    for value in mach {
        let (attribute, val) = match value.split_once('=') {
            Some((attribute, val)) => (attribute.to_string(), val.to_string()),
            None => ("CPUBOARD".to_string(), value.clone()),
        };
        builder = builder.add(&attribute, &val);
        pairs.push((attribute, val));
    }
    Ok((pairs, builder.build()))
}

/// Builds a path query: plain text matches as a substring, an explicit
/// `*`/`?` pattern is handed to the core wildcard matcher unchanged.
fn path_query(pattern: &str) -> Query {
    if pattern.contains(['*', '?']) {
        Query::path(pattern)
    } else {
        Query::path(format!("*{pattern}*"))
    }
}

/// Identifies an entry within a distribution, for set operations.
fn entry_key(entry: &Entry) -> (String, usize) {
    (entry.subsystem.product().to_string(), entry.id.0)
}

fn find_image<'a>(dist: &'a Distribution, name: &str) -> Option<(&'a Product, &'a Image)> {
    dist.products().iter().find_map(|product| {
        product
            .images
            .iter()
            .find(|image| image.name.to_string() == name)
            .map(|image| (product, image))
    })
}

fn find_subsystem<'a>(
    dist: &'a Distribution,
    name: &str,
) -> Option<(&'a Product, &'a Image, &'a Subsystem)> {
    dist.products().iter().find_map(|product| {
        product.images.iter().find_map(|image| {
            image
                .subsystems
                .iter()
                .find(|subsystem| subsystem.name.to_string() == name)
                .map(|subsystem| (product, image, subsystem))
        })
    })
}

fn products(dist: &Distribution) -> Result<()> {
    print!("{}", output::products_table(dist.products()));
    Ok(())
}

fn tree(dist: &Distribution, args: &TreeArgs) -> Result<()> {
    let product = dist
        .product(&args.product)
        .ok_or_else(|| CoreError::ProductNotFound {
            name: args.product.clone(),
        })?;
    print!("{}", output::product_tree(product));
    Ok(())
}

fn show(dist: &Distribution, args: &ShowArgs) -> Result<()> {
    match args.name.split('.').count() {
        1 => {
            let product = dist
                .product(&args.name)
                .ok_or_else(|| CoreError::ProductNotFound {
                    name: args.name.clone(),
                })?;
            print!("{}", output::product_details(product));
        }
        2 => {
            let (_product, image) = find_image(dist, &args.name)
                .ok_or_else(|| anyhow!("image not found: {}", args.name))?;
            print!("{}", output::image_details(image));
        }
        3 => {
            let (_product, image, subsystem) = find_subsystem(dist, &args.name)
                .ok_or_else(|| anyhow!("subsystem not found: {}", args.name))?;
            print!("{}", output::subsystem_details(image, subsystem));
        }
        // The argument parser already restricts the name to 1-3 segments.
        _ => unreachable!("target name arity is validated by the argument parser"),
    }
    Ok(())
}

fn find(dist: &Distribution, args: &FindArgs) -> Result<()> {
    let query = path_query(&args.pattern);
    let found = dist.find(&query);
    if args.mach.mach.is_empty() {
        if found.entries.is_empty() {
            eprintln!("no entries match {:?}", args.pattern);
        } else {
            print!("{}", output::entry_table(&found.entries));
        }
        return Ok(());
    }

    // With a hardware profile, the full hierarchical selection decides:
    // product, image, subsystem and entry restrictions all apply.
    let (_pairs, profile) = parse_mach(&args.mach.mach)?;
    let keys: HashSet<(String, usize)> =
        found.entries.iter().map(|entry| entry_key(entry)).collect();
    let selection = dist.select(&profile);
    let entries: Vec<&Entry> = selection
        .selected
        .iter()
        .filter(|entry| keys.contains(&entry_key(entry)))
        .copied()
        .collect();
    if entries.is_empty() {
        eprintln!(
            "no entries match {:?} for the given hardware profile",
            args.pattern
        );
    } else {
        print!("{}", output::entry_table(&entries));
    }
    let conflicts: Vec<&SelectionConflict> = selection
        .conflicts
        .iter()
        .filter(|conflict| {
            conflict
                .candidates
                .iter()
                .any(|entry| keys.contains(&entry_key(entry)))
        })
        .collect();
    if !conflicts.is_empty() {
        eprintln!("\n{} conflict(s) affect the result:", conflicts.len());
        eprint!("{}", output::conflict_report(&conflicts));
    }
    Ok(())
}

fn select(dist: &Distribution, args: &SelectArgs) -> Result<()> {
    let (pairs, profile) = parse_mach(&args.mach.mach)?;
    let selection = dist.select(&profile);
    let mut out = String::new();
    out.push_str("Hardware profile:\n");
    if pairs.is_empty() {
        out.push_str("  (none)\n");
    }
    for (attribute, value) in &pairs {
        let _ = writeln!(out, "  {attribute} = {value}");
    }
    out.push('\n');
    let _ = writeln!(out, "Selected entries: {}", selection.selected.len());
    let _ = writeln!(out, "Conflicts:        {}", selection.conflicts.len());
    if args.list {
        out.push('\n');
        for entry in &selection.selected {
            let _ = writeln!(out, "{}", entry.path);
        }
    }
    if !selection.conflicts.is_empty() {
        out.push('\n');
        let conflicts: Vec<&SelectionConflict> = selection.conflicts.iter().collect();
        out.push_str(&output::conflict_report(&conflicts));
    }
    print!("{out}");
    Ok(())
}

/// Resolves the extraction scope to a concrete entry list.
fn resolve_scope<'a>(dist: &'a Distribution, args: &ExtractArgs) -> Result<Vec<&'a Entry>> {
    if let Some(pattern) = &args.path {
        let entries = dist.find(&path_query(pattern)).entries;
        if entries.is_empty() {
            bail!("no entries match {pattern:?}");
        }
        return Ok(entries);
    }
    if let Some(name) = &args.product {
        let product = dist
            .product(name)
            .ok_or_else(|| CoreError::ProductNotFound { name: name.clone() })?;
        return Ok(product.entries.iter().collect());
    }
    if let Some(name) = &args.image {
        let (product, image) =
            find_image(dist, name).ok_or_else(|| anyhow!("image not found: {name}"))?;
        return Ok(product
            .entries
            .iter()
            .filter(|entry| entry.subsystem.image_name() == image.name)
            .collect());
    }
    if let Some(name) = &args.subsystem {
        let (product, _image, subsystem) =
            find_subsystem(dist, name).ok_or_else(|| anyhow!("subsystem not found: {name}"))?;
        return Ok(subsystem
            .entry_ids
            .iter()
            .filter_map(|id| product.entry(*id))
            .collect());
    }
    unreachable!("the argument parser requires exactly one scope");
}

/// The fail-safe gate every extraction goes through.
///
/// Extraction writes to the host filesystem, so unlike `find` (which may
/// show ambiguities) it must refuse to run whenever the entries to write
/// cannot be determined unambiguously.
fn gate_extraction<'a>(
    dist: &'a Distribution,
    entries: Vec<&'a Entry>,
    mach: &[String],
) -> Result<Vec<&'a Entry>> {
    if mach.is_empty() {
        gate_without_profile(dist, entries)
    } else {
        gate_with_profile(dist, entries, mach)
    }
}

/// With a hardware profile, the hierarchical selection decides; any
/// conflict or unresolved applicability touching the scope refuses the
/// whole extraction.
fn gate_with_profile<'a>(
    dist: &'a Distribution,
    entries: Vec<&'a Entry>,
    mach: &[String],
) -> Result<Vec<&'a Entry>> {
    let (_pairs, profile) = parse_mach(mach)?;
    let selection = dist.select(&profile);
    let scope: HashSet<(String, usize)> = entries.iter().map(|entry| entry_key(entry)).collect();
    let conflicts: Vec<&SelectionConflict> = selection
        .conflicts
        .iter()
        .filter(|conflict| {
            conflict
                .candidates
                .iter()
                .any(|entry| scope.contains(&entry_key(entry)))
        })
        .collect();
    if !conflicts.is_empty() {
        eprintln!("extraction refused; conflict(s) affect the requested entries:\n");
        eprint!("{}", output::conflict_report(&conflicts));
        bail!("extraction is ambiguous; no files were written");
    }
    let selected: HashSet<(String, usize)> = selection
        .selected
        .iter()
        .map(|entry| entry_key(entry))
        .collect();
    Ok(entries
        .into_iter()
        .filter(|entry| selected.contains(&entry_key(entry)))
        .collect())
}

/// Without a hardware profile no expression can be evaluated, so the
/// only safe scopes are those with a single variant per path and no
/// unresolvable applicability anywhere.
///
/// Duplicate directory entries are not ambiguity: several subsystems
/// legitimately create the same directory, matching the collision
/// preflight's dir/dir exemption.
fn gate_without_profile<'a>(
    dist: &'a Distribution,
    entries: Vec<&'a Entry>,
) -> Result<Vec<&'a Entry>> {
    let mut by_path: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for entry in &entries {
        by_path.entry(entry.path.as_str()).or_default().push(entry);
    }
    let ambiguous: Vec<&str> = by_path
        .iter()
        .filter(|(_, group)| {
            group.len() > 1
                && group
                    .iter()
                    .any(|entry| entry.file_type != FileType::Directory)
        })
        .map(|(path, _)| *path)
        .collect();
    let unparsed: Vec<&str> = entries
        .iter()
        .filter(|entry| entry.has_unparsed_mach())
        .map(|entry| entry.path.as_str())
        .collect();
    let unresolved = unresolved_subsystems(dist);
    let unknown: Vec<&str> = entries
        .iter()
        .filter(|entry| unresolved.contains(entry.subsystem.to_string().as_str()))
        .map(|entry| entry.path.as_str())
        .collect();

    if ambiguous.is_empty() && unparsed.is_empty() && unknown.is_empty() {
        return Ok(entries);
    }
    let mut message = String::from("extraction is ambiguous\n");
    for path in ambiguous.iter().take(5) {
        let _ = write!(message, "\n{path} has multiple applicable variants.");
    }
    if ambiguous.len() > 5 {
        let _ = write!(message, "\n... and {} more paths.", ambiguous.len() - 5);
    }
    if !unparsed.is_empty() {
        let _ = write!(
            message,
            "\n\nThe applicability of {} entr{} cannot be determined \
             (unparsable mach expression).",
            unparsed.len(),
            if unparsed.len() == 1 { "y" } else { "ies" }
        );
    }
    if !unknown.is_empty() {
        let _ = write!(
            message,
            "\n\nThe applicability of {} entr{} cannot be determined \
             (undecodable descriptor hardware restriction).",
            unknown.len(),
            if unknown.len() == 1 { "y" } else { "ies" }
        );
    }
    message.push_str("\n\nSpecify a hardware profile using --mach.");
    Err(anyhow!(message))
}

/// Names of the subsystems whose hardware applicability cannot be fully
/// decoded, at any level above them (product, image or subsystem).
fn unresolved_subsystems(dist: &Distribution) -> HashSet<String> {
    let mut unresolved = HashSet::new();
    let has_unresolved = |mach: &Option<sw_core::descriptor::model::HardwareRestrictions>| {
        mach.as_ref().is_some_and(|m| !m.unresolved.is_empty())
    };
    for product in dist.products() {
        let product_unresolved = has_unresolved(&product.mach);
        for image in &product.images {
            let image_unresolved = product_unresolved || has_unresolved(&image.mach);
            for subsystem in &image.subsystems {
                if image_unresolved || has_unresolved(&subsystem.mach) {
                    unresolved.insert(subsystem.name.to_string());
                }
            }
        }
    }
    unresolved
}

fn extract_command(dist: &Distribution, args: &ExtractArgs) -> Result<()> {
    let scope = resolve_scope(dist, args)?;
    if scope.is_empty() {
        bail!("the requested scope contains no entries");
    }

    let path_mode = if args.flat {
        PathMode::Flat
    } else if let Some(prefix) = &args.relative_to {
        PathMode::RelativeTo(prefix.clone())
    } else {
        PathMode::Full
    };
    let options = ExtractOptions {
        path_mode,
        decode: if args.raw {
            DecodeMode::Never
        } else {
            DecodeMode::Auto
        },
        keep_stored: args.keep_stored,
        continue_on_error: !args.fail_fast,
    };

    // Only entries that would actually produce output participate in the
    // fail-safe gate and the collision preflight: an entry that is never
    // written (e.g. outside a `--relative-to` prefix, or a device node)
    // cannot corrupt the output and must not block the extraction.
    //
    // Statically known failures (no payload, a path the host cannot
    // represent) are deferred: an entry the hardware selection discards
    // anyway must not abort the extraction either. The error propagates
    // only for entries that survive the gate, still before anything is
    // written.
    let projected: Vec<(&Entry, sw_core::error::Result<Vec<PathBuf>>)> = scope
        .iter()
        .map(|entry| (*entry, extract::output_paths(entry, &options)))
        .collect();
    let effective: Vec<&Entry> = projected
        .iter()
        .filter(|(_, paths)| !matches!(paths, Ok(paths) if paths.is_empty()))
        .map(|(entry, _)| *entry)
        .collect();
    if effective.is_empty() {
        bail!("no entries in the requested scope produce output with the given options");
    }
    let entries = gate_extraction(dist, effective, &args.mach.mach)?;
    if entries.is_empty() {
        bail!("no entries in scope apply to the given hardware profile");
    }

    let gated: HashSet<(String, usize)> = entries.iter().map(|entry| entry_key(entry)).collect();
    let mut gated_producers: Vec<(&Entry, Vec<PathBuf>)> = Vec::new();
    for (entry, paths) in projected {
        if gated.contains(&entry_key(entry)) {
            gated_producers.push((entry, paths?));
        }
    }
    preflight_output_collisions(&gated_producers)?;

    let total = entries.len();
    let mut reader = dist.image_reader();
    let report = extract::extract(&mut reader, &entries, &args.output, &options);
    print!("{}", output::extract_report(&report, total));
    if report.failures.is_empty() {
        Ok(())
    } else {
        bail!("{} entries failed to extract", report.failures.len())
    }
}

/// Refuses the extraction when two entries would write the same output
/// file.
///
/// The selection conflict machinery of `sw-core` arbitrates between
/// variants of one pathname *within* a subsystem; it does not cover
/// collisions that only appear once entries are mapped to host output
/// paths: different subsystems shipping the same pathname, `--flat`
/// merging different directories, or `--raw`/`--keep-stored` `.Z`
/// sidecars colliding with real files. Directory entries are exempt:
/// several subsystems legitimately create the same directory.
fn preflight_output_collisions(producers: &[(&Entry, Vec<PathBuf>)]) -> Result<()> {
    let mut by_output: BTreeMap<&Path, Vec<&Entry>> = BTreeMap::new();
    for (entry, paths) in producers {
        for path in paths {
            by_output.entry(path.as_path()).or_default().push(entry);
        }
    }
    let collisions: Vec<(&&Path, &Vec<&Entry>)> = by_output
        .iter()
        .filter(|(_, entries)| {
            entries.len() > 1
                && entries
                    .iter()
                    .any(|entry| entry.file_type != FileType::Directory)
        })
        .collect();
    if collisions.is_empty() {
        return Ok(());
    }
    let mut message = String::from("extraction would overwrite output files\n");
    for (path, entries) in collisions.iter().take(5) {
        let _ = write!(message, "\n{}", path.display());
        for entry in entries.iter() {
            let _ = write!(message, "\n  <- {}: {}", entry.subsystem, entry.path);
        }
    }
    if collisions.len() > 5 {
        let _ = write!(message, "\n... and {} more outputs.", collisions.len() - 5);
    }
    message.push_str("\n\nNo files were written.");
    Err(anyhow!(message))
}
