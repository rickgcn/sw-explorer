//! Descriptor corpus analyzer.
//!
//! Scans one or more distribution directories and emits a unified
//! JSON/CSV report describing every product descriptor as raw, located
//! data:
//!
//! * header bytes, file size and SHA-256 (whole file and body only) of
//!   each descriptor file;
//! * the IDB-derived image / subsystem hierarchy of each product;
//! * every byte offset at which a known product, image or subsystem name
//!   occurs in a descriptor body (matched against the union of names of
//!   the whole corpus, so cross-product references are located too),
//!   classified into top-level occurrences and occurrences fully
//!   contained in a longer known-name occurrence;
//! * every maximal run of printable bytes in each descriptor body;
//! * the context bytes around the first occurrence of each product's
//!   own name.
//!
//! The tool performs no field interpretation: it reports where known
//! names and strings are, never what the surrounding bytes mean.
//!
//! Usage:
//!
//! ```sh
//! cargo run -p sw-core --example descriptor-corpus -- \
//!     --json report.json --csv report.csv --samples samples/ <dist-dir>...
//! ```

use aho_corasick::AhoCorasick;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::ExitCode;
use sw_core::distribution::Distribution;

/// Descriptor header length in bytes, including the trailing NUL.
const HEADER_SIZE: usize = 13;

/// Name kind labels used in the report.
const KIND_PRODUCT: &str = "product";
const KIND_IMAGE: &str = "image";
const KIND_SUBSYSTEM: &str = "subsystem";

/// The whole corpus report.
#[derive(Serialize)]
struct CorpusReport {
    tool: String,
    distributions: Vec<DistReport>,
    /// Corpus-wide registry of every known name and the kinds it
    /// appears as.
    names: BTreeMap<String, Vec<String>>,
}

/// One scanned distribution directory.
#[derive(Serialize)]
struct DistReport {
    path: String,
    /// Fatal open error, if the directory could not be read at all.
    open_error: Option<String>,
    products: Vec<ProductReport>,
}

/// One product: its descriptor file, IDB-derived hierarchy and the
/// located name occurrences.
#[derive(Serialize)]
struct ProductReport {
    name: String,
    descriptor: DescriptorReport,
    /// Context bytes around the first occurrence of the product's own
    /// name in the body.
    self_name: Option<SelfNameContext>,
    hierarchy: Vec<ImageReport>,
    /// Located names, keyed by name; only names that occur at least
    /// once are listed.
    occurrences: BTreeMap<String, Occurrence>,
    /// Every maximal run of printable bytes in the body.
    strings: Vec<StringRun>,
    diagnostics: Vec<String>,
}

/// Raw facts about one descriptor file.
#[derive(Serialize)]
struct DescriptorReport {
    path: String,
    present: bool,
    size_bytes: u64,
    sha256: Option<String>,
    /// SHA-256 of the body only (everything after the header).
    body_sha256: Option<String>,
    /// The header bytes as hex (up to [`HEADER_SIZE`] bytes).
    header_hex: Option<String>,
    /// Whether the file starts with the `pd001V` magic.
    magic_ok: bool,
    /// Whether the header's last byte is NUL.
    nul_terminated: bool,
    body_size: u64,
}

/// One image of the IDB-derived hierarchy.
#[derive(Serialize)]
struct ImageReport {
    name: String,
    subsystems: Vec<SubsystemReport>,
}

/// One subsystem of the IDB-derived hierarchy.
#[derive(Serialize)]
struct SubsystemReport {
    name: String,
    entries: usize,
}

/// Context bytes around the first occurrence of a product's own name.
#[derive(Serialize)]
struct SelfNameContext {
    body_offset: u64,
    /// Every body byte before the name.
    prefix_hex: String,
    /// Up to 128 body bytes starting at the name.
    context_hex: String,
}

/// One maximal run of printable bytes in a descriptor body.
#[derive(Serialize)]
struct StringRun {
    body_offset: u64,
    length: u64,
    text: String,
}

/// All body offsets at which one name occurs.
#[derive(Serialize)]
struct Occurrence {
    kinds: Vec<String>,
    count: usize,
    body_offsets: Vec<u64>,
    /// Offsets not contained in any longer known-name occurrence.
    top_level_offsets: Vec<u64>,
    /// Offsets fully inside a longer known-name occurrence; these are
    /// usually substring artifacts (e.g. `eoe` inside `ViewKit_eoe`)
    /// rather than independent fields.
    contained_offsets: Vec<u64>,
}

/// Encodes a string one byte per character, mirroring the Latin-1
/// convention used for names inside distribution files.
fn latin1_bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u32 as u8).collect()
}

/// Decodes bytes into a string, one character per byte.
fn latin1_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Lowercase hex encoding.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes lowercase hex produced by [`hex`].
fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0))
        .collect()
}

/// Minimal command line: `--json <path>` / `--csv <path>` plus any
/// number of distribution directories.
struct Args {
    json: Option<PathBuf>,
    csv: Option<PathBuf>,
    samples: Option<PathBuf>,
    min_string_len: usize,
    dists: Vec<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        json: None,
        csv: None,
        samples: None,
        min_string_len: 2,
        dists: Vec::new(),
    };
    let mut argv = std::env::args_os().skip(1);
    while let Some(arg) = argv.next() {
        match arg.to_str() {
            Some("--json") => {
                args.json = Some(PathBuf::from(argv.next().ok_or("--json needs a path")?));
            }
            Some("--csv") => {
                args.csv = Some(PathBuf::from(argv.next().ok_or("--csv needs a path")?));
            }
            Some("--samples") => {
                args.samples = Some(PathBuf::from(
                    argv.next().ok_or("--samples needs a directory")?,
                ));
            }
            Some("--min-string-len") => {
                let value = argv.next().ok_or("--min-string-len needs a number")?;
                args.min_string_len = value
                    .to_str()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--min-string-len needs a number")?;
            }
            Some("--help") => return Err("help requested".to_string()),
            Some(flag) if flag.starts_with('-') => {
                return Err(format!("unknown option: {flag}"));
            }
            _ => args.dists.push(PathBuf::from(arg)),
        }
    }
    if args.dists.is_empty() {
        return Err("no distribution directories given".to_string());
    }
    Ok(args)
}

/// Registers one name under a kind.
fn register(registry: &mut BTreeMap<String, BTreeSet<String>>, kind: &str, name: String) {
    registry.entry(name).or_default().insert(kind.to_string());
}

/// Builds the IDB-derived hierarchy report of one product and registers
/// all its names.
fn build_hierarchy(
    product: &sw_core::distribution::Product,
    registry: &mut BTreeMap<String, BTreeSet<String>>,
) -> Vec<ImageReport> {
    register(registry, KIND_PRODUCT, product.name.as_str().to_string());
    let mut images = Vec::new();
    for image in &product.images {
        register(registry, KIND_IMAGE, image.name.to_string());
        let mut subsystems = Vec::new();
        for subsystem in &image.subsystems {
            register(registry, KIND_SUBSYSTEM, subsystem.name.to_string());
            subsystems.push(SubsystemReport {
                name: subsystem.name.to_string(),
                entries: subsystem.entry_ids.len(),
            });
        }
        images.push(ImageReport {
            name: image.name.to_string(),
            subsystems,
        });
    }
    images
}

/// Reports the raw facts of one descriptor file from its bytes.
fn describe_descriptor(path: &std::path::Path, bytes: &[u8]) -> DescriptorReport {
    DescriptorReport {
        path: path.display().to_string(),
        present: true,
        size_bytes: bytes.len() as u64,
        sha256: Some(hex(&Sha256::digest(bytes))),
        body_sha256: Some(hex(&Sha256::digest(&bytes[bytes.len().min(HEADER_SIZE)..]))),
        header_hex: Some(hex(&bytes[..bytes.len().min(HEADER_SIZE)])),
        magic_ok: bytes.starts_with(b"pd001V"),
        nul_terminated: bytes.len() >= HEADER_SIZE && bytes[HEADER_SIZE - 1] == 0,
        body_size: bytes.len().saturating_sub(HEADER_SIZE) as u64,
    }
}

/// Reports an absent descriptor file.
fn missing_descriptor(path: &std::path::Path) -> DescriptorReport {
    DescriptorReport {
        path: path.display().to_string(),
        present: false,
        size_bytes: 0,
        sha256: None,
        body_sha256: None,
        header_hex: None,
        magic_ok: false,
        nul_terminated: false,
        body_size: 0,
    }
}

/// Locates every known name in a descriptor body.
fn locate_names(
    body: &[u8],
    automaton: &AhoCorasick,
    patterns: &[Vec<u8>],
    registry: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, Occurrence> {
    let mut occurrences: BTreeMap<String, Occurrence> = BTreeMap::new();
    for m in automaton.find_overlapping_iter(body) {
        let name = String::from_utf8_lossy(&patterns[m.pattern().as_usize()]).into_owned();
        let occurrence = occurrences
            .entry(name.clone())
            .or_insert_with(|| Occurrence {
                kinds: registry
                    .get(&name)
                    .map(|k| k.iter().cloned().collect())
                    .unwrap_or_default(),
                count: 0,
                body_offsets: Vec::new(),
                top_level_offsets: Vec::new(),
                contained_offsets: Vec::new(),
            });
        occurrence.count += 1;
        occurrence.body_offsets.push(m.start() as u64);
    }
    occurrences
}

/// Splits every occurrence's offsets into top-level ones and ones fully
/// contained in a longer known-name occurrence. Names are one byte per
/// character, so character count equals interval length.
fn classify_occurrences(occurrences: &mut BTreeMap<String, Occurrence>) {
    let mut intervals: Vec<(u64, u64)> = Vec::new();
    for (name, occurrence) in occurrences.iter() {
        let len = name.chars().count() as u64;
        for &start in &occurrence.body_offsets {
            intervals.push((start, start + len));
        }
    }

    for (name, occurrence) in occurrences.iter_mut() {
        let len = name.chars().count() as u64;
        for &start in &occurrence.body_offsets {
            let end = start + len;
            let contained = intervals
                .iter()
                .any(|&(s, e)| s <= start && e >= end && e - s > len);
            if contained {
                occurrence.contained_offsets.push(start);
            } else {
                occurrence.top_level_offsets.push(start);
            }
        }
    }
}

/// Captures the context bytes around the first occurrence of a
/// product's own name in the body.
fn self_name_context(product_name: &str, body: &[u8]) -> Option<SelfNameContext> {
    let needle = latin1_bytes(product_name);
    if needle.is_empty() || needle.len() > body.len() {
        return None;
    }
    let offset = body.windows(needle.len()).position(|w| w == needle)?;
    Some(SelfNameContext {
        body_offset: offset as u64,
        prefix_hex: hex(&body[..offset]),
        context_hex: hex(&body[offset..body.len().min(offset + 128)]),
    })
}

/// Finds every maximal run of printable bytes (space..`~` plus the
/// Latin-1 supplement) of at least `min_len` bytes.
fn scan_strings(body: &[u8], min_len: usize) -> Vec<StringRun> {
    let is_string_byte = |b: u8| (0x20..=0x7e).contains(&b) || b >= 0xa0;
    let mut runs = Vec::new();
    let mut start: Option<usize> = None;
    for (index, &byte) in body.iter().enumerate() {
        if is_string_byte(byte) {
            start.get_or_insert(index);
        } else if let Some(s) = start.take()
            && index - s >= min_len
        {
            runs.push(StringRun {
                body_offset: s as u64,
                length: (index - s) as u64,
                text: latin1_string(&body[s..index]),
            });
        }
    }
    if let Some(s) = start
        && body.len() - s >= min_len
    {
        runs.push(StringRun {
            body_offset: s as u64,
            length: (body.len() - s) as u64,
            text: latin1_string(&body[s..]),
        });
    }
    runs
}

/// Escapes one CSV field.
fn csv_field(out: &mut String, value: &str) {
    if value.contains([',', '"', '\n']) {
        out.push('"');
        out.push_str(&value.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(value);
    }
}

/// Copies the smallest descriptor of every distinct header version into
/// a sample directory, for close manual inspection. Returns the copied
/// file names.
fn write_samples(report: &CorpusReport, dir: &std::path::Path) -> std::io::Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;

    // header hex -> (size, source path, product name)
    let mut smallest: BTreeMap<&str, (u64, &str, &str)> = BTreeMap::new();
    for dist in &report.distributions {
        for product in &dist.products {
            let d = &product.descriptor;
            let (true, Some(header)) = (d.present, &d.header_hex) else {
                continue;
            };
            let entry = smallest
                .entry(header.as_str())
                .or_insert((u64::MAX, "", ""));
            if d.size_bytes < entry.0 {
                *entry = (d.size_bytes, d.path.as_str(), product.name.as_str());
            }
        }
    }

    let mut copied = Vec::new();
    for (header, (_, source, product)) in &smallest {
        let header_bytes = unhex(header);
        let magic = latin1_string(&header_bytes)
            .trim_end_matches('\0')
            .to_string();
        let file_name = format!("{magic}-{product}");
        std::fs::copy(source, dir.join(&file_name))?;
        copied.push(file_name);
    }
    Ok(copied)
}

/// Renders the unified flat CSV: one row per descriptor, per hierarchy
/// node, per located name offset and per printable string run.
fn render_csv(report: &CorpusReport) -> String {
    const COLUMNS: usize = 15;
    let mut out = String::from(
        "record,dist,product,image,kind,name,entries,descriptor_path,size_bytes,sha256,body_sha256,header_hex,body_offset,file_offset,length\n",
    );

    for dist in &report.distributions {
        for product in &dist.products {
            let mut rows: Vec<[String; COLUMNS]> = Vec::new();
            let row = |record: &str| {
                let mut fields: [String; COLUMNS] = Default::default();
                fields[0] = record.to_string();
                fields[1] = dist.path.clone();
                fields[2] = product.name.clone();
                fields
            };

            let d = &product.descriptor;
            let mut fields = row("descriptor");
            fields[7] = d.path.clone();
            fields[8] = d.size_bytes.to_string();
            fields[9] = d.sha256.clone().unwrap_or_default();
            fields[10] = d.body_sha256.clone().unwrap_or_default();
            fields[11] = d.header_hex.clone().unwrap_or_default();
            rows.push(fields);

            for image in &product.hierarchy {
                let mut fields = row("image");
                fields[5] = image.name.clone();
                rows.push(fields);
                for subsystem in &image.subsystems {
                    let mut fields = row("subsystem");
                    fields[3] = image.name.clone();
                    fields[5] = subsystem.name.clone();
                    fields[6] = subsystem.entries.to_string();
                    rows.push(fields);
                }
            }

            for (name, occurrence) in &product.occurrences {
                let length = name.chars().count().to_string();
                for (record, offsets) in [
                    ("occurrence", &occurrence.top_level_offsets),
                    ("occurrence_contained", &occurrence.contained_offsets),
                ] {
                    for offset in offsets {
                        let mut fields = row(record);
                        fields[4] = occurrence.kinds.join("|");
                        fields[5] = name.clone();
                        fields[12] = offset.to_string();
                        fields[13] = (offset + HEADER_SIZE as u64).to_string();
                        fields[14] = length.clone();
                        rows.push(fields);
                    }
                }
            }

            for string in &product.strings {
                let mut fields = row("string");
                fields[5] = string.text.clone();
                fields[12] = string.body_offset.to_string();
                fields[13] = (string.body_offset + HEADER_SIZE as u64).to_string();
                fields[14] = string.length.to_string();
                rows.push(fields);
            }

            for fields in &rows {
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    csv_field(&mut out, field);
                }
                out.push('\n');
            }
        }
    }
    out
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!(
                "usage: descriptor-corpus [--json <path>] [--csv <path>] [--samples <dir>] [--min-string-len <n>] <dist-dir>..."
            );
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };

    // Pass 1: open every distribution and build the corpus-wide name
    // registry, so cross-product references are located as well.
    let mut dists: Vec<(String, Option<Distribution>, Vec<Vec<ImageReport>>)> = Vec::new();
    let mut registry: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut open_failures = 0usize;
    for path in &args.dists {
        match Distribution::open(path) {
            Ok(dist) => {
                let hierarchies = dist
                    .products()
                    .iter()
                    .map(|p| build_hierarchy(p, &mut registry))
                    .collect();
                dists.push((path.display().to_string(), Some(dist), hierarchies));
            }
            Err(error) => {
                eprintln!("warning: cannot open {}: {error}", path.display());
                dists.push((path.display().to_string(), None, Vec::new()));
                open_failures += 1;
            }
        }
    }
    if dists.iter().all(|(_, d, _)| d.is_none()) {
        eprintln!("error: no distribution could be opened");
        return ExitCode::FAILURE;
    }

    let patterns: Vec<Vec<u8>> = registry.keys().map(|n| latin1_bytes(n)).collect();
    let automaton = match AhoCorasick::new(&patterns) {
        Ok(a) => a,
        Err(error) => {
            eprintln!("error: cannot build name matcher: {error}");
            return ExitCode::FAILURE;
        }
    };

    // Pass 2: describe every descriptor and locate all known names in
    // its body.
    let mut distributions = Vec::new();
    for (path, dist, hierarchies) in dists {
        let mut products = Vec::new();
        if let Some(dist) = &dist {
            for (product, hierarchy) in dist.products().iter().zip(hierarchies) {
                let bytes = std::fs::read(&product.descriptor_file).ok();
                let (descriptor, occurrences, self_name, strings) = match &bytes {
                    Some(bytes) => {
                        let body = &bytes[bytes.len().min(HEADER_SIZE)..];
                        let mut occurrences = locate_names(body, &automaton, &patterns, &registry);
                        classify_occurrences(&mut occurrences);
                        (
                            describe_descriptor(&product.descriptor_file, bytes),
                            occurrences,
                            self_name_context(product.name.as_str(), body),
                            scan_strings(body, args.min_string_len),
                        )
                    }
                    None => (
                        missing_descriptor(&product.descriptor_file),
                        BTreeMap::new(),
                        None,
                        Vec::new(),
                    ),
                };
                let diagnostics = product
                    .diagnostics
                    .iter()
                    .map(|d| {
                        let origin = d
                            .origin
                            .as_ref()
                            .map(|o| format!(" ({o})"))
                            .unwrap_or_default();
                        format!("{:?}: {}{origin}", d.severity, d.message)
                    })
                    .collect();
                products.push(ProductReport {
                    name: product.name.as_str().to_string(),
                    descriptor,
                    self_name,
                    hierarchy,
                    occurrences,
                    strings,
                    diagnostics,
                });
            }
        }
        distributions.push(DistReport {
            path,
            open_error: if dist.is_none() {
                Some("failed to open distribution".to_string())
            } else {
                None
            },
            products,
        });
    }

    let report = CorpusReport {
        tool: format!("sw-core descriptor-corpus {}", env!("CARGO_PKG_VERSION")),
        distributions,
        names: registry
            .iter()
            .map(|(n, k)| (n.clone(), k.iter().cloned().collect()))
            .collect(),
    };

    let json = match serde_json::to_string_pretty(&report) {
        Ok(json) => json,
        Err(error) => {
            eprintln!("error: cannot serialize report: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(path) = &args.json {
        if let Err(error) = std::fs::write(path, &json) {
            eprintln!("error: cannot write {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
    } else if args.csv.is_none() {
        println!("{json}");
    }
    if let Some(path) = &args.csv
        && let Err(error) = std::fs::write(path, render_csv(&report))
    {
        eprintln!("error: cannot write {}: {error}", path.display());
        return ExitCode::FAILURE;
    }
    if let Some(dir) = &args.samples {
        match write_samples(&report, dir) {
            Ok(copied) => eprintln!(
                "copied {} smallest-per-header samples to {}: {}",
                copied.len(),
                dir.display(),
                copied.join(", ")
            ),
            Err(error) => {
                eprintln!("error: cannot write samples to {}: {error}", dir.display());
                return ExitCode::FAILURE;
            }
        }
    }

    let products: usize = report.distributions.iter().map(|d| d.products.len()).sum();
    let matches: usize = report
        .distributions
        .iter()
        .flat_map(|d| &d.products)
        .flat_map(|p| p.occurrences.values())
        .map(|o| o.count)
        .sum();
    eprintln!(
        "scanned {} products in {} distributions; {} known names; {matches} name occurrences located",
        products,
        report.distributions.len(),
        report.names.len(),
    );
    if open_failures > 0 {
        eprintln!("warning: {open_failures} distribution(s) could not be opened");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
