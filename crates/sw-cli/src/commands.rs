//! Command implementations: call `sw-core`, gather results, render them.
//!
//! No IRIX format logic lives here; every question is answered through
//! the public `sw-core` API.

use crate::cli::{Cli, Command, ExtractArgs, FindArgs, SelectArgs, ShowArgs, TreeArgs};
use crate::output;
use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashSet;
use std::fmt::Write as _;
use sw_core::descriptor::model::Subsystem;
use sw_core::diagnostic::Severity;
use sw_core::distribution::{Distribution, EntryKey, LocatedEntry, Product};
use sw_core::error::Error as CoreError;
use sw_core::extract::{DecodeMode, ExistingOutputPolicy, ExtractOptions, PathMode};
use sw_core::image::Image;
use sw_core::mach::eval::HardwareProfile;
use sw_core::plan;
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
    for diagnostic in dist.all_diagnostics() {
        let severity = match diagnostic.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        match &diagnostic.origin {
            Some(origin) => eprintln!("{severity}: {} ({origin})", diagnostic.message),
            None => eprintln!("{severity}: {}", diagnostic.message),
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

/// Identifies an entry's owning product by name, with its index.
fn find_product<'a>(dist: &'a Distribution, name: &str) -> Option<(usize, &'a Product)> {
    dist.products()
        .iter()
        .enumerate()
        .find(|(_, product)| product.name.as_str() == name)
}

fn find_image<'a>(dist: &'a Distribution, name: &str) -> Option<(usize, &'a Product, &'a Image)> {
    dist.products()
        .iter()
        .enumerate()
        .find_map(|(index, product)| {
            product
                .images
                .iter()
                .find(|image| image.name.to_string() == name)
                .map(|image| (index, product, image))
        })
}

fn find_subsystem<'a>(
    dist: &'a Distribution,
    name: &str,
) -> Option<(usize, &'a Product, &'a Image, &'a Subsystem)> {
    dist.products()
        .iter()
        .enumerate()
        .find_map(|(index, product)| {
            product.images.iter().find_map(|image| {
                image
                    .subsystems
                    .iter()
                    .find(|subsystem| subsystem.name.to_string() == name)
                    .map(|subsystem| (index, product, image, subsystem))
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
            let (_index, _product, image) = find_image(dist, &args.name)
                .ok_or_else(|| anyhow!("image not found: {}", args.name))?;
            print!("{}", output::image_details(image));
        }
        3 => {
            let (_index, _product, image, subsystem) = find_subsystem(dist, &args.name)
                .ok_or_else(|| anyhow!("subsystem not found: {}", args.name))?;
            print!("{}", output::subsystem_details(image, subsystem));
        }
        // The argument parser already restricts the name to 1-3 segments.
        _ => unreachable!("target name arity is validated by the argument parser"),
    }
    Ok(())
}

fn find(dist: &Distribution, args: &FindArgs) -> Result<()> {
    let query = Query::path_search(&args.pattern)?;
    let found = dist.find(&query);
    if args.mach.mach.is_empty() {
        if found.entries.is_empty() {
            eprintln!("no entries match {:?}", args.pattern);
        } else {
            print!("{}", output::entry_table(dist, &found.entries));
        }
        return Ok(());
    }

    // With a hardware profile, the full hierarchical selection decides:
    // product, image, subsystem and entry restrictions all apply.
    let (_pairs, profile) = parse_mach(&args.mach.mach)?;
    let keys: HashSet<EntryKey> = found.entries.iter().map(|located| located.key).collect();
    let selection = dist.select(&profile);
    let entries: Vec<LocatedEntry> = selection
        .selected
        .iter()
        .filter(|located| keys.contains(&located.key))
        .copied()
        .collect();
    if entries.is_empty() {
        eprintln!(
            "no entries match {:?} for the given hardware profile",
            args.pattern
        );
    } else {
        print!("{}", output::entry_table(dist, &entries));
    }
    let conflicts: Vec<&SelectionConflict> = selection
        .conflicts
        .iter()
        .filter(|conflict| {
            conflict
                .candidates
                .iter()
                .any(|located| keys.contains(&located.key))
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
        for located in &selection.selected {
            let _ = writeln!(out, "{}", located.entry.path);
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

/// Resolves the extraction scope to canonical entry keys; the shared
/// planner resolves them against the distribution itself.
fn resolve_scope(dist: &Distribution, args: &ExtractArgs) -> Result<Vec<EntryKey>> {
    if let Some(pattern) = &args.path {
        let keys: Vec<EntryKey> = dist
            .find(&Query::path_search(pattern)?)
            .entries
            .into_iter()
            .map(|located| located.key)
            .collect();
        if keys.is_empty() {
            bail!("no entries match {pattern:?}");
        }
        return Ok(keys);
    }
    if let Some(name) = &args.product {
        let (product_index, product) = find_product(dist, name)
            .ok_or_else(|| CoreError::ProductNotFound { name: name.clone() })?;
        return Ok(product
            .entries
            .iter()
            .map(|entry| EntryKey {
                product_index,
                entry_id: entry.id,
            })
            .collect());
    }
    if let Some(name) = &args.image {
        let (product_index, product, image) =
            find_image(dist, name).ok_or_else(|| anyhow!("image not found: {name}"))?;
        return Ok(product
            .entries
            .iter()
            .filter(|entry| entry.subsystem.image_name() == image.name)
            .map(|entry| EntryKey {
                product_index,
                entry_id: entry.id,
            })
            .collect());
    }
    if let Some(name) = &args.subsystem {
        let (product_index, product, _image, subsystem) =
            find_subsystem(dist, name).ok_or_else(|| anyhow!("subsystem not found: {name}"))?;
        return Ok(subsystem
            .entry_ids
            .iter()
            .filter(|id| product.entry(**id).is_some())
            .map(|id| EntryKey {
                product_index,
                entry_id: *id,
            })
            .collect());
    }
    unreachable!("the argument parser requires exactly one scope");
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
        // The CLI keeps its historical overwrite behavior; the shared
        // planner still refuses symbolic-link and other unsafe targets.
        existing_output: ExistingOutputPolicy::Allow,
    };
    let profile = if args.mach.mach.is_empty() {
        None
    } else {
        Some(parse_mach(&args.mach.mach)?.1)
    };

    // All extraction safety — hardware selection, ambiguity, output
    // collisions, output topology and existing outputs — lives in the
    // shared sw-core planner, which re-evaluates everything immediately
    // before anything is written.
    let checked = plan::extract_checked(dist, &scope, &args.output, &options, profile.as_ref())?;
    print!(
        "{}",
        output::extract_report(&checked.report, checked.plan.planned_records)
    );
    if checked.report.failures.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} entries failed to extract",
            checked.report.failures.len()
        )
    }
}
