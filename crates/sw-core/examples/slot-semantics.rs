//! Corpus statistics for the subsystem rule slots whose semantics are
//! not yet confirmed (slots 2, 5, 7 and 8).
//!
//! For every non-empty instance the report records the full context
//! (distribution, product, image, subsystem, layout level, raw subsystem
//! flags, image version) and the slot contents. Aggregate statistics
//! per slot cover layout-level and flag-word distributions, target and
//! version-bound histograms for range slots, and prefix / hardware
//! expression feature counts for string slots.
//!
//! Everything is derived from the library's descriptor parser; nothing
//! here guesses record boundaries.
//!
//! Usage:
//!
//! ```sh
//! cargo run -p sw-core --example slot-semantics -- --json slots.json <dist-dir>...
//! ```

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use sw_core::descriptor::model::{DescriptorRange, RuleSlot};
use sw_core::distribution::Distribution;
use sw_core::mach::HardwareExpr;

/// One serialized range record of a slot instance.
#[derive(Serialize)]
struct RangeOut {
    target: String,
    low: u64,
    /// Raw high bound; `0x7fffffff` is SGI `maxint` (unbounded).
    high: u64,
}

/// One non-empty slot of one subsystem.
#[derive(Serialize)]
struct SlotInstance {
    dist: String,
    product: String,
    image: String,
    subsystem: String,
    layout_level: u16,
    flags_raw: u16,
    image_version: u64,
    slot: u8,
    kind: String,
    ranges: Vec<RangeOut>,
    strings: Vec<String>,
}

/// Aggregate statistics of one slot index.
#[derive(Serialize, Default)]
struct SlotStats {
    /// Subsystems with a non-empty instance of this slot.
    instances: usize,
    /// Total records across all instances.
    records: usize,
    by_layout_level: BTreeMap<String, usize>,
    by_flags: BTreeMap<String, usize>,
    /// Instances whose product name starts with `patch` or `maint`.
    patch_product_instances: usize,
    // Range slots.
    /// Most frequent range targets.
    top_targets: BTreeMap<String, usize>,
    low_zero: usize,
    high_is_version_minus_one: usize,
    high_is_maxint: usize,
    self_target: usize,
    patch_star_target: usize,
    maint_star_target: usize,
    // String slots.
    /// First character of each string.
    first_chars: BTreeMap<String, usize>,
    /// Distinct example strings, capped.
    examples: Vec<String>,
    /// Strings the MACH parser accepts.
    mach_parses: usize,
    contains_cpuboard: usize,
    contains_cpuarch: usize,
    contains_gfxboard: usize,
    contains_mode_eq: usize,
    contains_and: usize,
    contains_or: usize,
}

impl SlotStats {
    fn observe(&mut self, instance: &SlotInstance) {
        self.instances += 1;
        *self
            .by_layout_level
            .entry(instance.layout_level.to_string())
            .or_default() += 1;
        *self
            .by_flags
            .entry(format!("{:#06x}", instance.flags_raw))
            .or_default() += 1;
        if instance.product.starts_with("patch") || instance.product.starts_with("maint") {
            self.patch_product_instances += 1;
        }
        self.records += instance.ranges.len() + instance.strings.len();
        let self_target = format!(
            "{}.{}.{}",
            instance.product, instance.image, instance.subsystem
        );
        for range in &instance.ranges {
            *self.top_targets.entry(range.target.clone()).or_default() += 1;
            if range.low == 0 {
                self.low_zero += 1;
            }
            if range.high + 1 == instance.image_version {
                self.high_is_version_minus_one += 1;
            }
            if range.high == 0x7fff_ffff {
                self.high_is_maxint += 1;
            }
            if range.target == self_target {
                self.self_target += 1;
            }
            if range.target.starts_with("patch*") {
                self.patch_star_target += 1;
            }
            if range.target.starts_with("maint*") {
                self.maint_star_target += 1;
            }
        }
        for string in &instance.strings {
            let first = string
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_default();
            *self.first_chars.entry(first).or_default() += 1;
            if !self.examples.contains(string) && self.examples.len() < 25 {
                self.examples.push(string.clone());
            }
            if HardwareExpr::parse(string).is_ok() {
                self.mach_parses += 1;
            }
            if string.contains("CPUBOARD") {
                self.contains_cpuboard += 1;
            }
            if string.contains("CPUARCH") {
                self.contains_cpuarch += 1;
            }
            if string.contains("GFXBOARD") {
                self.contains_gfxboard += 1;
            }
            if string.contains("MODE=") {
                self.contains_mode_eq += 1;
            }
            if string.contains("&&") {
                self.contains_and += 1;
            }
            if string.contains("||") {
                self.contains_or += 1;
            }
        }
    }
}

/// The whole report.
#[derive(Serialize)]
struct Report {
    tool: String,
    /// Subsystem flag-word distribution across *all* subsystems, as the
    /// baseline for per-slot flag histograms.
    baseline_flags: BTreeMap<String, usize>,
    instances: Vec<SlotInstance>,
    stats: BTreeMap<String, SlotStats>,
}

fn range_out(range: &DescriptorRange) -> RangeOut {
    RangeOut {
        target: range.target(),
        low: u64::from(range.low_raw),
        high: u64::from(range.high_raw),
    }
}

fn main() -> ExitCode {
    let mut json: Option<PathBuf> = None;
    let mut dists: Vec<PathBuf> = Vec::new();
    let mut argv = std::env::args_os().skip(1);
    let arg_error = |message: &str| {
        eprintln!("usage: slot-semantics [--json <path>] <dist-dir>...");
        eprintln!("error: {message}");
        ExitCode::FAILURE
    };
    while let Some(arg) = argv.next() {
        match arg.to_str() {
            Some("--json") => match argv.next() {
                Some(path) => json = Some(PathBuf::from(path)),
                None => return arg_error("--json needs a path"),
            },
            Some(flag) if flag.starts_with('-') => {
                return arg_error(&format!("unknown option: {flag}"));
            }
            _ => dists.push(PathBuf::from(arg)),
        }
    }
    if dists.is_empty() {
        eprintln!("usage: slot-semantics [--json <path>] <dist-dir>...");
        return ExitCode::FAILURE;
    }

    let mut baseline_flags: BTreeMap<String, usize> = BTreeMap::new();
    let mut instances = Vec::new();
    let mut stats: BTreeMap<String, SlotStats> = BTreeMap::new();

    for path in &dists {
        let dist = match Distribution::open(path) {
            Ok(dist) => dist,
            Err(error) => {
                eprintln!("warning: cannot open {}: {error}", path.display());
                continue;
            }
        };
        for product in dist.products() {
            let Some(descriptor) = &product.descriptor else {
                eprintln!("warning: {}: no parsed descriptor, skipped", product.name);
                continue;
            };
            for image in &descriptor.images {
                for subsystem in &image.subsystems {
                    *baseline_flags
                        .entry(format!("{:#06x}", subsystem.flags_raw))
                        .or_default() += 1;
                    for slot in &subsystem.unassigned_slots {
                        let (kind, ranges, strings) = match slot {
                            RuleSlot::Ranges { values, .. } => {
                                ("ranges", values.iter().map(range_out).collect(), Vec::new())
                            }
                            RuleSlot::Strings { values, .. } => {
                                ("strings", Vec::new(), values.clone())
                            }
                        };
                        let instance = SlotInstance {
                            dist: path.display().to_string(),
                            product: descriptor.name.clone(),
                            image: image.name.clone(),
                            subsystem: subsystem.name.clone(),
                            layout_level: descriptor.layout_level,
                            flags_raw: subsystem.flags_raw,
                            image_version: image.version.0,
                            slot: slot.slot(),
                            kind: kind.to_string(),
                            ranges,
                            strings,
                        };
                        stats
                            .entry(slot.slot().to_string())
                            .or_default()
                            .observe(&instance);
                        instances.push(instance);
                    }
                }
            }
        }
    }

    eprintln!("{} non-empty slot instances", instances.len());
    for (slot, stat) in &stats {
        eprintln!(
            "  slot {slot}: {} instances, {} records, levels {:?}",
            stat.instances,
            stat.records,
            stat.by_layout_level.keys().collect::<Vec<_>>()
        );
    }

    let report = Report {
        tool: format!("sw-core slot-semantics {}", env!("CARGO_PKG_VERSION")),
        baseline_flags,
        instances,
        stats,
    };
    let json_text = match serde_json::to_string_pretty(&report) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot serialize report: {error}");
            return ExitCode::FAILURE;
        }
    };
    match json {
        Some(path) => {
            if let Err(error) = std::fs::write(&path, json_text) {
                eprintln!("error: cannot write {}: {error}", path.display());
                return ExitCode::FAILURE;
            }
        }
        None => println!("{json_text}"),
    }
    ExitCode::SUCCESS
}
