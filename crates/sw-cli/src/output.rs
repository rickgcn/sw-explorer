//! Human-readable rendering of `sw-core` results.

use std::fmt::Write as _;
use sw_core::descriptor::model::{
    ConditionalFlag, HardwareRestrictions, Subsystem, SubsystemRange, VersionLimit,
};
use sw_core::distribution::Product;
use sw_core::extract::ExtractReport;
use sw_core::idb::attribute::IdbAttribute;
use sw_core::idb::{Entry, FileType};
use sw_core::image::{Image, PayloadResolution};
use sw_core::selection::SelectionConflict;

/// Renders an aligned table. The last column is left unpadded.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.len());
        }
    }
    let mut out = String::new();
    let render = |cells: &[String], out: &mut String| {
        let last = cells.len() - 1;
        for (index, cell) in cells.iter().enumerate() {
            if index == last {
                let _ = write!(out, "{cell}");
            } else {
                let _ = write!(out, "{cell:<width$}  ", width = widths[index]);
            }
        }
        out.push('\n');
    };
    let header_cells: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
    render(&header_cells, &mut out);
    for row in rows {
        render(row, &mut out);
    }
    out
}

/// Renders the product listing table.
pub fn products_table(products: &[Product]) -> String {
    let rows: Vec<Vec<String>> = products
        .iter()
        .map(|product| {
            vec![
                product.name.to_string(),
                yes_no(product.descriptor.is_some()),
                yes_no(product.idb_file.is_some()),
                product.images.len().to_string(),
                product
                    .images
                    .iter()
                    .map(|image| image.subsystems.len())
                    .sum::<usize>()
                    .to_string(),
                product.entries.len().to_string(),
                product.title.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    table(
        &["NAME", "D", "I", "IMAGES", "SUBSYSTEMS", "ENTRIES", "TITLE"],
        &rows,
    )
}

fn yes_no(value: bool) -> String {
    if value { "Y" } else { "N" }.to_string()
}

/// Lowercase yes/no used by the detail views.
fn word(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// Renders the product / image / subsystem tree of one product.
pub fn product_tree(product: &Product) -> String {
    let mut out = String::new();
    let title = product.title.as_deref().unwrap_or("-");
    let _ = writeln!(out, "{} — {}", product.name, title);
    for (index, image) in product.images.iter().enumerate() {
        let last_image = index + 1 == product.images.len();
        let branch = if last_image {
            "└── "
        } else {
            "├── "
        };
        let prefix = if last_image { "    " } else { "│   " };
        let _ = writeln!(out, "{branch}{}", image.name);
        let version = image
            .version
            .map(|version| version.0.to_string())
            .unwrap_or_else(|| "-".to_string());
        let order = image
            .order
            .map(|order| order.to_string())
            .unwrap_or_else(|| "-".to_string());
        let _ = writeln!(out, "{prefix}version {version}   order {order}");
        for (sub_index, subsystem) in image.subsystems.iter().enumerate() {
            let last_subsystem = sub_index + 1 == image.subsystems.len();
            let sub_branch = if last_subsystem {
                "└── "
            } else {
                "├── "
            };
            let sub_prefix = if last_subsystem {
                format!("{prefix}    ")
            } else {
                format!("{prefix}│   ")
            };
            let _ = writeln!(out, "{prefix}{sub_branch}{}", subsystem.name);
            let _ = writeln!(out, "{sub_prefix}{} entries", subsystem.entry_ids.len());
            let flags = tree_flags(subsystem);
            if !flags.is_empty() {
                let _ = writeln!(out, "{sub_prefix}{}", flags.join(" "));
            }
        }
    }
    out
}

/// Short flag words shown next to a subsystem in the tree.
fn tree_flags(subsystem: &Subsystem) -> Vec<String> {
    let mut words = Vec::new();
    let Some(flags) = &subsystem.flags else {
        return words;
    };
    for (name, flag) in [
        ("required", &flags.required),
        ("default", &flags.default),
        ("miniroot", &flags.miniroot),
    ] {
        match flag {
            ConditionalFlag::No => {}
            ConditionalFlag::Always => words.push(name.to_string()),
            ConditionalFlag::When(_) => words.push(format!("{name}(conditional)")),
            ConditionalFlag::WhenUnresolved { .. } => {
                words.push(format!("{name}(unresolved)"));
            }
        }
    }
    if flags.patch {
        words.push("patch".to_string());
    }
    if flags.client_only {
        words.push("client-only".to_string());
    }
    if flags.overlay {
        words.push("overlay".to_string());
    }
    if flags.overlay_member {
        words.push("overlay-member".to_string());
    }
    words
}

/// Renders the details of a product.
pub fn product_details(product: &Product) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Product: {}", product.name);
    let _ = writeln!(out, "Title:   {}", product.title.as_deref().unwrap_or("-"));
    let _ = writeln!(out, "Descriptor: {}", word(product.descriptor.is_some()));
    let _ = writeln!(out, "IDB:        {}", word(product.idb_file.is_some()));
    out.push('\n');
    push_mach_section(&mut out, &product.mach);
    out.push('\n');
    let _ = writeln!(out, "Images:");
    if product.images.is_empty() {
        let _ = writeln!(out, "  -");
    }
    for image in &product.images {
        let _ = writeln!(out, "  {}", image.name);
    }
    if !product.cutpoints.is_empty() {
        out.push('\n');
        let _ = writeln!(out, "Cutpoints:");
        for cutpoint in &product.cutpoints {
            let _ = writeln!(out, "  {} (sequence {})", cutpoint.path, cutpoint.sequence);
        }
    }
    out
}

/// Renders the details of an image.
pub fn image_details(image: &Image) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Image:   {}", image.name);
    let _ = writeln!(out, "Title:   {}", image.title.as_deref().unwrap_or("-"));
    let version = image
        .version
        .map(|version| version.0.to_string())
        .unwrap_or_else(|| "-".to_string());
    let _ = writeln!(out, "Version: {version}");
    let order = image
        .order
        .map(|order| order.to_string())
        .unwrap_or_else(|| "-".to_string());
    let _ = writeln!(out, "Order:   {order}");
    out.push('\n');
    push_mach_section(&mut out, &image.mach);
    out.push('\n');
    let _ = writeln!(out, "Subsystems:");
    if image.subsystems.is_empty() {
        let _ = writeln!(out, "  -");
    }
    for subsystem in &image.subsystems {
        let _ = writeln!(out, "  {}", subsystem.name);
    }
    out
}

/// Renders the details of a subsystem.
pub fn subsystem_details(image: &Image, subsystem: &Subsystem) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Subsystem: {}", subsystem.name);
    let _ = writeln!(
        out,
        "Title:     {}",
        subsystem.title.as_deref().unwrap_or("-")
    );
    let _ = writeln!(out, "Image:     {}", image.name);
    let version = image
        .version
        .map(|version| version.0.to_string())
        .unwrap_or_else(|| "-".to_string());
    let _ = writeln!(out, "Version:   {version}");
    out.push('\n');
    let _ = writeln!(out, "Presence:");
    let _ = writeln!(out, "  descriptor: {}", word(subsystem.presence.descriptor));
    let _ = writeln!(out, "  IDB:        {}", word(subsystem.presence.idb));
    out.push('\n');
    let _ = writeln!(out, "Flags:");
    match &subsystem.flags {
        Some(flags) => {
            let _ = writeln!(out, "  required: {}", conditional_flag(&flags.required));
            let _ = writeln!(out, "  default:  {}", conditional_flag(&flags.default));
            let _ = writeln!(out, "  miniroot: {}", conditional_flag(&flags.miniroot));
            let _ = writeln!(out, "  patch:    {}", word(flags.patch));
            let _ = writeln!(out, "  inplace:  {}", word(flags.inplace));
            let _ = writeln!(out, "  client-only:    {}", word(flags.client_only));
            let _ = writeln!(out, "  overlay:        {}", word(flags.overlay));
            let _ = writeln!(out, "  overlay-member: {}", word(flags.overlay_member));
        }
        None => {
            let _ = writeln!(out, "  unknown (no descriptor record)");
        }
    }
    out.push('\n');
    push_mach_section(&mut out, &subsystem.mach);
    out.push('\n');
    let _ = writeln!(out, "Mapping:");
    let _ = writeln!(out, "  {}", subsystem.mapping.as_deref().unwrap_or("-"));
    out.push('\n');
    let _ = writeln!(out, "Entries: {}", subsystem.entry_ids.len());
    out.push('\n');
    match &subsystem.rules {
        Some(rules) => {
            let _ = writeln!(out, "Prerequisites:");
            if rules.prerequisites.is_empty() {
                let _ = writeln!(out, "  -");
            }
            for clause in &rules.prerequisites {
                let ranges: Vec<String> = clause.all_of.iter().map(range_display).collect();
                let _ = writeln!(out, "  {}", ranges.join(" + "));
            }
            out.push('\n');
            push_range_section(&mut out, "Replaces", &rules.replaces);
            out.push('\n');
            push_range_section(&mut out, "Incompatible", &rules.incompatibilities);
            out.push('\n');
            push_range_section(&mut out, "Updates", &rules.updates);
            out.push('\n');
            push_range_section(&mut out, "Follows", &rules.follows);
        }
        None => {
            let _ = writeln!(out, "Rules: unknown (no descriptor record)");
        }
    }
    out.push('\n');
    let _ = writeln!(out, "Autominiroot:");
    match &subsystem.autominiroot {
        Some(ranges) => {
            if ranges.is_empty() {
                let _ = writeln!(out, "  -");
            }
            for range in ranges {
                let _ = writeln!(out, "  {}", range_display(range));
            }
        }
        None => {
            let _ = writeln!(out, "  unknown (no descriptor record)");
        }
    }
    out
}

/// Renders a hardware restrictions section, including a prominent
/// "Unresolved MACH" section for expressions that could not be parsed.
fn push_mach_section(out: &mut String, mach: &Option<HardwareRestrictions>) {
    let _ = writeln!(out, "MACH:");
    match mach {
        None => {
            let _ = writeln!(out, "  unknown (no descriptor record)");
        }
        Some(restrictions) => {
            if restrictions.expressions.is_empty() {
                let _ = writeln!(out, "  -");
            }
            for expression in &restrictions.expressions {
                let _ = writeln!(out, "  {}", expression.raw);
            }
            if !restrictions.unresolved.is_empty() {
                out.push('\n');
                let _ = writeln!(out, "Unresolved MACH:");
                for raw in &restrictions.unresolved {
                    let _ = writeln!(out, "  {raw}");
                }
            }
        }
    }
}

fn push_range_section(out: &mut String, title: &str, ranges: &[SubsystemRange]) {
    let _ = writeln!(out, "{title}:");
    if ranges.is_empty() {
        let _ = writeln!(out, "  -");
    }
    for range in ranges {
        let _ = writeln!(out, "  {}", range_display(range));
    }
}

/// Renders a conditional flag, distinguishing plain, conditional and
/// unresolved conditions.
fn conditional_flag(flag: &ConditionalFlag) -> String {
    match flag {
        ConditionalFlag::No => "no".to_string(),
        ConditionalFlag::Always => "yes".to_string(),
        ConditionalFlag::When(expressions) => {
            format!("when {}", join_expressions(expressions))
        }
        ConditionalFlag::WhenUnresolved {
            expressions,
            unresolved,
        } => {
            if expressions.is_empty() {
                format!("unresolved condition: {}", unresolved.join(", "))
            } else {
                format!(
                    "when {} (unresolved condition: {})",
                    join_expressions(expressions),
                    unresolved.join(", ")
                )
            }
        }
    }
}

fn join_expressions(expressions: &[sw_core::mach::HardwareExpr]) -> String {
    expressions
        .iter()
        .map(|expression| expression.raw.clone())
        .collect::<Vec<_>>()
        .join(" or ")
}

/// Renders a subsystem rule range, e.g. `patch*.sw.unix 0..1274627332`.
fn range_display(range: &SubsystemRange) -> String {
    let max = match range.versions.max {
        VersionLimit::Exact(version) => version.0.to_string(),
        VersionLimit::Maximum => "maxint".to_string(),
    };
    format!("{} {}..{}", range.target(), range.versions.min.0, max)
}

/// Renders the entry search result table.
pub fn entry_table(entries: &[&Entry]) -> String {
    let rows: Vec<Vec<String>> = entries
        .iter()
        .map(|entry| {
            vec![
                entry.subsystem.product().to_string(),
                entry.subsystem.to_string(),
                file_type_name(entry.file_type).to_string(),
                entry
                    .size()
                    .map(|size| size.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                entry
                    .encoded_size()
                    .map(|size| size.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                entry_mach(entry),
                entry.path.to_string(),
            ]
        })
        .collect();
    table(
        &[
            "PRODUCT",
            "SUBSYSTEM",
            "TYPE",
            "SIZE",
            "STORED",
            "MACH",
            "PATH",
        ],
        &rows,
    )
}

fn file_type_name(file_type: FileType) -> &'static str {
    match file_type {
        FileType::Regular => "file",
        FileType::Directory => "dir",
        FileType::SymbolicLink => "link",
        FileType::BlockDevice => "block",
        FileType::CharacterDevice => "char",
        FileType::Fifo => "fifo",
        FileType::Unknown(_) => "other",
    }
}

/// The hardware expressions of an entry for display, including
/// unparsable ones (which must never look like "no restriction").
pub fn entry_mach(entry: &Entry) -> String {
    let mut parts = Vec::new();
    for attribute in &entry.attributes {
        match attribute {
            IdbAttribute::Mach(expression) => parts.push(expression.raw.clone()),
            IdbAttribute::MachUnparsed { raw, .. } => {
                parts.push(format!("unparsable: {raw}"));
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        "-".to_string()
    } else {
        parts.join(", ")
    }
}

/// Renders selection conflicts: the contested path plus each candidate.
pub fn conflict_report(conflicts: &[&SelectionConflict]) -> String {
    let mut out = String::new();
    for conflict in conflicts {
        let _ = writeln!(out, "CONFLICT {}", conflict.path);
        for candidate in &conflict.candidates {
            out.push('\n');
            let _ = writeln!(out, "  {}", candidate.subsystem);
            let _ = writeln!(out, "  mach: {}", entry_mach(candidate));
        }
        out.push('\n');
    }
    out
}

/// Renders the extraction summary, surfacing payload recoveries and
/// failures instead of hiding them in the counts.
pub fn extract_report(report: &ExtractReport, total: usize) -> String {
    let mut out = String::new();
    out.push_str("\nExtraction complete\n\n");
    let _ = writeln!(out, "Total:      {total}");
    let _ = writeln!(out, "Extracted:  {}", report.extracted);
    let _ = writeln!(out, "Skipped:    {}", report.skipped);
    let _ = writeln!(out, "Errors:     {}", report.failures.len());
    let _ = writeln!(out, "Recoveries: {}", report.recoveries.len());
    if !report.recoveries.is_empty() {
        out.push_str("\nRecovered payload:\n");
        for recovery in &report.recoveries {
            let _ = writeln!(out, "  {}", recovery.path);
            let expected = recovery
                .expected_record_offset
                .map(|offset| format!("0x{offset:x}"))
                .unwrap_or_else(|| "-".to_string());
            let _ = writeln!(out, "  expected: {expected}");
            let _ = writeln!(out, "  actual:   0x{:x}", recovery.actual_record_offset);
            let method = match recovery.resolution {
                PayloadResolution::Exact => "exact",
                PayloadResolution::Delta { .. } => "delta",
                PayloadResolution::Resynced { .. } => "resync",
                PayloadResolution::Scanned => "scan",
            };
            let _ = writeln!(out, "  method:   {method}");
        }
    }
    if !report.failures.is_empty() {
        out.push_str("\nFailures:\n");
        for failure in &report.failures {
            let _ = writeln!(out, "  {}: {}", failure.path, failure.message);
        }
    }
    out
}
