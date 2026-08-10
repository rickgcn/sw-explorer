//! Extraction planning: the shared fail-safe gate every extraction goes
//! through before any bytes are written.
//!
//! Both frontends (the CLI and the Qt GUI) hand concrete entry lists to
//! [`plan_extraction`] and execute through [`extract_checked`]; neither
//! re-implements hardware selection, collision detection or
//! existing-output handling. The planner performs its checks in this
//! exact order:
//!
//! 1. Project every requested entry to its output paths, holding static
//!    projection errors back instead of aborting.
//! 2. Drop deliberate omissions (entries producing no output at all:
//!    path-mode exclusions, devices, FIFOs, symbolic links without a
//!    recorded target).
//! 3. Run the hardware gate: with a profile the hierarchical selection
//!    decides and any conflict touching the effective scope refuses the
//!    plan; without a profile any ambiguity in the effective scope
//!    refuses it.
//! 4. Propagate the held projection errors, but only for entries that
//!    survived the gate: a statically bad entry the hardware selection
//!    excludes anyway must not block the extraction.
//! 5. Refuse output collisions, unsafe output topology, unsafe existing
//!    ancestors, and existing output paths the options disallow.
//!
//! Planning never mutates the filesystem: it only inspects paths and
//! metadata, so a plan can be presented for confirmation before anything
//! is written. These checks close the usual application-level hazards
//! (a symlinked component below the output root, an unexpected existing
//! file, two entries writing one output); they do not attempt to defend
//! against a hostile process concurrently mutating the output tree.
use crate::descriptor::model::HardwareRestrictions;
use crate::distribution::Distribution;
use crate::error::{Error, Result};
use crate::extract::{self, ExistingOutputPolicy, ExtractOptions, ExtractReport};
use crate::idb::attribute::IdbAttribute;
use crate::idb::{Entry, FileType};
use crate::mach::eval::HardwareProfile;
use crate::selection::SelectionConflict;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Counts describing a planned extraction, for user confirmation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractionPlanSummary {
    /// Entries the frontend handed to the planner.
    pub requested_records: usize,
    /// Entries that deliberately produce no output (path-mode
    /// exclusions, devices, FIFOs, symbolic links without a recorded
    /// target).
    pub omitted_records: usize,
    /// Entries that would produce output but were excluded by the
    /// hardware profile selection. Zero without a profile.
    pub hardware_excluded_records: usize,
    /// Entries that will be handed to the extraction.
    pub planned_records: usize,
    /// Projected output paths of the planned entries; one entry can
    /// produce two paths (`keep_stored` adds a `.Z` sidecar).
    pub output_paths: usize,
    /// Existing regular files that will be overwritten under
    /// [`ExistingOutputPolicy::Allow`].
    pub existing_outputs: usize,
}

/// A confirmed extraction plan.
///
/// `entries` is the authoritative execution list, in requested order;
/// frontends must not re-derive it from the summary.
#[derive(Debug)]
pub struct ExtractionPlan<'a> {
    /// Entries to extract, after omissions and the hardware gate.
    pub entries: Vec<&'a Entry>,
    /// Counts describing the plan.
    pub summary: ExtractionPlanSummary,
}

/// The result of a checked extraction: the plan that was executed plus
/// the extraction outcome.
#[derive(Debug)]
pub struct CheckedExtractReport {
    /// The plan confirmed immediately before writing.
    pub plan: ExtractionPlanSummary,
    /// The extraction outcome.
    pub report: ExtractReport,
}

/// Plans an extraction without touching the filesystem.
///
/// `requested` is the frontend's scope already resolved to concrete
/// entries; the planner knows nothing about object ids, search modes or
/// command-line flags. `out_dir` need not exist; planning never creates
/// it. All refusals are reported as [`Error::ExtractionPlan`] except
/// static projection errors ([`Error::UnsafePath`],
/// [`Error::PayloadNotFound`]), which keep their original type.
///
/// # Errors
///
/// Returns an error whenever the extraction cannot be run safely:
/// ambiguity without a profile, hardware conflicts or an empty
/// selection with a profile, static projection failures of selected
/// entries, output collisions, unsafe output topology, unsafe existing
/// ancestors, existing outputs the policy disallows, or an output root
/// that exists as a non-directory.
pub fn plan_extraction<'a>(
    distribution: &'a Distribution,
    requested: &[&'a Entry],
    out_dir: &Path,
    options: &ExtractOptions,
    profile: Option<&HardwareProfile>,
) -> Result<ExtractionPlan<'a>> {
    // Project every requested entry first, holding errors back: an
    // entry the hardware gate discards anyway must not abort the plan.
    let projected: Vec<(&Entry, Result<Vec<PathBuf>>)> = requested
        .iter()
        .map(|entry| (*entry, extract::output_paths(entry, options)))
        .collect();

    // Entries that deliberately produce nothing (`Ok([])`) never reach
    // the gate: they cannot corrupt the output and must not block the
    // extraction. Static failures (`Err`) stay effective.
    let omitted = projected
        .iter()
        .filter(|(_, paths)| matches!(paths, Ok(paths) if paths.is_empty()))
        .count();
    let effective: Vec<&Entry> = projected
        .iter()
        .filter(|(_, paths)| !matches!(paths, Ok(paths) if paths.is_empty()))
        .map(|(entry, _)| *entry)
        .collect();
    if effective.is_empty() {
        return Err(plan_error(
            "no entries in the requested scope produce output with the given options",
        ));
    }

    // The hardware gate. Without a profile nothing is excluded; the
    // gate only ever refuses.
    let (gated, hardware_excluded) = match profile {
        Some(profile) => gate_with_profile(distribution, &effective, profile)?,
        None => (gate_without_profile(distribution, &effective)?, 0),
    };

    // Propagate held projection errors, but only for entries that
    // survived the gate, still before anything is written.
    let gated_set: HashSet<*const Entry> = gated
        .iter()
        .map(|entry| std::ptr::from_ref(*entry))
        .collect();
    let mut producers: Vec<(&Entry, Vec<PathBuf>)> = Vec::new();
    for (entry, paths) in projected {
        if gated_set.contains(&std::ptr::from_ref(entry)) {
            producers.push((entry, paths?));
        }
    }

    preflight_output_collisions(&producers)?;
    preflight_output_topology(&producers)?;
    preflight_output_root(out_dir)?;
    preflight_existing_ancestors(out_dir, &producers)?;
    let existing_outputs =
        preflight_existing_outputs(out_dir, &producers, options.existing_output)?;

    let output_paths = producers.iter().map(|(_, paths)| paths.len()).sum();
    let entries: Vec<&Entry> = producers.iter().map(|(entry, _)| *entry).collect();
    let planned_records = entries.len();
    Ok(ExtractionPlan {
        entries,
        summary: ExtractionPlanSummary {
            requested_records: requested.len(),
            omitted_records: omitted,
            hardware_excluded_records: hardware_excluded,
            planned_records,
            output_paths,
            existing_outputs,
        },
    })
}

/// Plans and immediately executes an extraction.
///
/// The plan is recomputed here, immediately before writing: anything
/// that changed on the host filesystem or in the inputs since a
/// caller's earlier [`plan_extraction`] is re-evaluated, and a refusal
/// means zero writes. Only when the fresh plan succeeds are the planned
/// entries extracted.
///
/// # Errors
///
/// Returns the [`plan_extraction`] errors before anything is written.
/// Runtime failures of individual entries are reported through
/// [`CheckedExtractReport::report`] instead.
pub fn extract_checked<'a>(
    distribution: &'a Distribution,
    requested: &[&'a Entry],
    out_dir: &Path,
    options: &ExtractOptions,
    profile: Option<&HardwareProfile>,
) -> Result<CheckedExtractReport> {
    let plan = plan_extraction(distribution, requested, out_dir, options, profile)?;
    let mut reader = distribution.image_reader();
    let report = extract::extract(&mut reader, &plan.entries, out_dir, options);
    Ok(CheckedExtractReport {
        plan: plan.summary,
        report,
    })
}

/// Without a hardware profile no expression can be evaluated, so the
/// only safe scopes are those with a single variant per path and no
/// unresolvable applicability anywhere.
///
/// Duplicate directory entries are not ambiguity: several subsystems
/// legitimately create the same directory, matching the collision
/// preflight's dir/dir exemption.
fn gate_without_profile<'a>(
    distribution: &'a Distribution,
    effective: &[&'a Entry],
) -> Result<Vec<&'a Entry>> {
    let mut by_path: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for entry in effective {
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
    let unparsed: Vec<&str> = effective
        .iter()
        .filter(|entry| entry.has_unparsed_mach())
        .map(|entry| entry.path.as_str())
        .collect();
    let unresolved = unresolved_entries(distribution);
    let unknown: Vec<&str> = effective
        .iter()
        .filter(|entry| unresolved.contains(&std::ptr::from_ref(*entry)))
        .map(|entry| entry.path.as_str())
        .collect();

    if ambiguous.is_empty() && unparsed.is_empty() && unknown.is_empty() {
        return Ok(effective.to_vec());
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
            entry_plural(unparsed.len())
        );
    }
    if !unknown.is_empty() {
        let _ = write!(
            message,
            "\n\nThe applicability of {} entr{} cannot be determined \
             (undecodable descriptor hardware restriction).",
            unknown.len(),
            entry_plural(unknown.len())
        );
    }
    message.push_str("\n\nSpecify a hardware profile.");
    Err(plan_error(message))
}

/// `y`/`ies` pluralization for the ambiguity message.
fn entry_plural(count: usize) -> &'static str {
    if count == 1 { "y" } else { "ies" }
}

/// Entries whose hardware applicability cannot be fully decoded because
/// a descriptor level above them (product, image or subsystem) contains
/// an undecodable hardware restriction.
///
/// Membership is by entry identity, never by subsystem name: a name is
/// not an ownership boundary.
fn unresolved_entries(distribution: &Distribution) -> HashSet<*const Entry> {
    let mut affected = HashSet::new();
    let has_unresolved = |mach: &Option<HardwareRestrictions>| {
        mach.as_ref().is_some_and(|m| !m.unresolved.is_empty())
    };
    for product in distribution.products() {
        let product_unresolved = has_unresolved(&product.mach);
        for image in &product.images {
            let image_unresolved = product_unresolved || has_unresolved(&image.mach);
            for subsystem in &image.subsystems {
                if image_unresolved || has_unresolved(&subsystem.mach) {
                    for id in &subsystem.entry_ids {
                        if let Some(entry) = product.entry(*id) {
                            affected.insert(std::ptr::from_ref(entry));
                        }
                    }
                }
            }
        }
    }
    affected
}

/// With a hardware profile the hierarchical selection decides; any
/// conflict or unresolved applicability touching the effective scope
/// refuses the whole plan. Conflicts entirely outside the scope do not
/// block it. Returns the surviving entries plus how many effective
/// entries the selection excluded.
fn gate_with_profile<'a>(
    distribution: &'a Distribution,
    effective: &[&'a Entry],
    profile: &HardwareProfile,
) -> Result<(Vec<&'a Entry>, usize)> {
    let selection = distribution.select(profile);
    let scope: HashSet<*const Entry> = effective
        .iter()
        .map(|entry| std::ptr::from_ref(*entry))
        .collect();
    let conflicts: Vec<&SelectionConflict> = selection
        .conflicts
        .iter()
        .filter(|conflict| {
            conflict
                .candidates
                .iter()
                .any(|entry| scope.contains(&std::ptr::from_ref(*entry)))
        })
        .collect();
    if !conflicts.is_empty() {
        return Err(plan_error(conflict_message(&conflicts)));
    }
    let selected: HashSet<*const Entry> = selection
        .selected
        .iter()
        .map(|entry| std::ptr::from_ref(*entry))
        .collect();
    let gated: Vec<&Entry> = effective
        .iter()
        .filter(|entry| selected.contains(&std::ptr::from_ref(**entry)))
        .copied()
        .collect();
    if gated.is_empty() {
        return Err(plan_error(
            "no entries in scope apply to the given hardware profile",
        ));
    }
    let excluded = effective.len() - gated.len();
    Ok((gated, excluded))
}

/// The refusal message for hardware conflicts, carrying the contested
/// paths and their candidates.
fn conflict_message(conflicts: &[&SelectionConflict]) -> String {
    let mut message = String::from("extraction is ambiguous; no files were written\n");
    let _ = write!(
        message,
        "\n{} hardware conflict(s) affect the requested entries:\n",
        conflicts.len()
    );
    for conflict in conflicts {
        let _ = write!(message, "\nCONFLICT {}", conflict.path);
        for candidate in &conflict.candidates {
            let _ = write!(message, "\n  {}", candidate.subsystem);
            let _ = write!(message, "\n  mach: {}", entry_mach(candidate));
        }
    }
    message
}

/// The hardware expressions of an entry for display, including
/// unparsable ones (which must never look like "no restriction").
fn entry_mach(entry: &Entry) -> String {
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

/// Host-aware comparison key for a planned output path.
///
/// Two planned outputs differing only in letter case are distinct
/// `Path`s to Rust, but the host filesystem decides whether they name
/// the same object: Windows volumes are case-insensitive, so `Foo` and
/// `foo` collide there. Planned outputs are therefore compared through
/// this key (lowercased on Windows); error messages keep the original
/// paths. Components are normalized first so the key comparison matches
/// `Path` equality everywhere else.
fn output_path_key(path: &Path) -> String {
    let normalized: PathBuf = path.components().collect();
    let text = normalized.to_string_lossy();
    #[cfg(windows)]
    {
        text.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        text.into_owned()
    }
}

/// Refuses the plan when two different entries would write the same
/// output path.
///
/// The selection machinery arbitrates between variants of one pathname
/// *within* a subsystem; it does not cover collisions that only appear
/// once entries are mapped to host output paths: different subsystems
/// shipping the same pathname, flat mode merging different directories,
/// or raw/keep-stored `.Z` sidecars colliding with real files.
/// Directory entries are exempt: several subsystems legitimately create
/// the same directory.
fn preflight_output_collisions(producers: &[(&Entry, Vec<PathBuf>)]) -> Result<()> {
    let mut by_output: BTreeMap<String, (&Path, Vec<&Entry>)> = BTreeMap::new();
    for (entry, paths) in producers {
        for path in paths {
            let (_, entries) = by_output
                .entry(output_path_key(path))
                .or_insert_with(|| (path.as_path(), Vec::new()));
            if !entries.iter().any(|other| std::ptr::eq(*other, *entry)) {
                entries.push(entry);
            }
        }
    }
    let collisions: Vec<(&Path, &Vec<&Entry>)> = by_output
        .values()
        .filter(|(_, entries)| {
            entries.len() > 1
                && entries
                    .iter()
                    .any(|entry| entry.file_type != FileType::Directory)
        })
        .map(|(path, entries)| (*path, entries))
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
    Err(plan_error(message))
}

/// Refuses outputs that would be written *through* another planned
/// output that is not a directory.
///
/// A planned symbolic link or regular file that is also an ancestor of
/// another planned output would redirect that output away from the
/// output directory: `usr -> /etc` followed by `usr/passwd` must never
/// be written. Planned directory ancestors are the normal case and stay
/// legal.
fn preflight_output_topology(producers: &[(&Entry, Vec<PathBuf>)]) -> Result<()> {
    // Every planned output path and whether it is a directory output,
    // keyed for the host filesystem (see `output_path_key`); the
    // original paths are kept for messages. Collisions were already
    // refused, so a key has a single kind here (duplicated directory
    // outputs agree on it).
    let mut kinds: HashMap<String, (&Path, bool)> = HashMap::new();
    for (entry, paths) in producers {
        let is_directory = entry.file_type == FileType::Directory;
        for path in paths {
            if path.as_os_str().is_empty() && !is_directory {
                // A non-directory entry mapping to the output root
                // itself would replace the output directory with a
                // file.
                return Err(plan_error(format!(
                    "invalid output topology\n\n{}: {} maps to the output directory itself\n\nNo files were written.",
                    entry.subsystem, entry.path
                )));
            }
            kinds.insert(output_path_key(path), (path.as_path(), is_directory));
        }
    }
    let mut blocked: Vec<(&Path, &Path)> = Vec::new();
    for (path, _) in kinds.values() {
        for ancestor in path.ancestors().skip(1) {
            if ancestor.as_os_str().is_empty() {
                continue;
            }
            if let Some((original, is_directory)) = kinds.get(&output_path_key(ancestor))
                && !is_directory
            {
                blocked.push((*original, *path));
            }
        }
    }
    blocked.sort();
    blocked.dedup();
    if blocked.is_empty() {
        return Ok(());
    }
    let mut message = String::from("extraction would write through a non-directory output\n");
    for (ancestor, path) in blocked.iter().take(5) {
        let _ = write!(
            message,
            "\n{} is not a directory but is an ancestor of {}",
            ancestor.display(),
            path.display()
        );
    }
    if blocked.len() > 5 {
        let _ = write!(message, "\n... and {} more.", blocked.len() - 5);
    }
    message.push_str("\n\nNo files were written.");
    Err(plan_error(message))
}

/// The output root is the user's explicit trust boundary: it may itself
/// be a symbolic link, and it need not exist yet (planning never
/// creates it). It only must not exist as a non-directory.
fn preflight_output_root(out_dir: &Path) -> Result<()> {
    match std::fs::metadata(out_dir) {
        Ok(metadata) if !metadata.is_dir() => Err(plan_error(format!(
            "output directory {} exists and is not a directory",
            out_dir.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::io(out_dir, source)),
    }
}

/// Every ancestor component below the output root that already exists
/// must be a real directory. A symbolic link (or regular or special
/// file) ancestor would redirect writes outside the output root:
/// `out/usr -> /somewhere` must never let `out/usr/foo` through.
///
/// [`std::fs::symlink_metadata`] is used deliberately: anything that
/// follows symbolic links would misjudge exactly the case being
/// defended.
fn preflight_existing_ancestors(
    out_dir: &Path,
    producers: &[(&Entry, Vec<PathBuf>)],
) -> Result<()> {
    let mut checked: HashSet<PathBuf> = HashSet::new();
    for (_, paths) in producers {
        for path in paths {
            for ancestor in path.ancestors().skip(1) {
                if ancestor.as_os_str().is_empty() {
                    continue;
                }
                let host = out_dir.join(ancestor);
                if !checked.insert(host.clone()) {
                    continue;
                }
                match std::fs::symlink_metadata(&host) {
                    Ok(metadata) if metadata.is_dir() => {}
                    Ok(_) => {
                        return Err(plan_error(format!(
                            "extraction would write through an existing non-directory\n\n{} already exists and is not a plain directory\n\nNo files were written.",
                            host.display()
                        )));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(source) => return Err(Error::io(&host, source)),
                }
            }
        }
    }
    Ok(())
}

/// Classifies planned output targets that already exist.
///
/// A directory output merging into an existing real directory is always
/// legal. Everything else depends on the policy:
/// [`ExistingOutputPolicy::Refuse`] rejects any existing target, while
/// [`ExistingOutputPolicy::Allow`] only permits a *regular-file output*
/// replacing an existing *regular file* — never a symbolic link in
/// either direction (a naive write would follow an existing link to
/// somewhere else, and a Unix symlink output replacing a file would
/// change the filesystem object type rather than overwrite contents),
/// never a directory, never a special file. On Windows a symlink entry
/// projects to a regular breadcrumb file and overwrites as one.
///
/// Returns the number of existing regular files that will be replaced.
fn preflight_existing_outputs(
    out_dir: &Path,
    producers: &[(&Entry, Vec<PathBuf>)],
    policy: ExistingOutputPolicy,
) -> Result<usize> {
    let mut overwrites: Vec<PathBuf> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();
    let mut existing_regular = 0;
    for (entry, paths) in producers {
        for path in paths {
            let host = out_dir.join(path);
            let metadata = match std::fs::symlink_metadata(&host) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => return Err(Error::io(&host, source)),
            };
            if entry.file_type == FileType::Directory {
                if metadata.is_dir() {
                    continue;
                }
                conflicts.push(format!(
                    "{} exists and is not a directory; the directory {}: {} cannot replace it",
                    host.display(),
                    entry.subsystem,
                    entry.path
                ));
                continue;
            }
            match policy {
                ExistingOutputPolicy::Refuse => overwrites.push(host),
                ExistingOutputPolicy::Allow => {
                    if metadata.file_type().is_symlink() {
                        conflicts.push(format!(
                            "{} is an existing symbolic link; refusing to follow it",
                            host.display()
                        ));
                    } else if metadata.is_dir() {
                        conflicts.push(format!(
                            "{} is an existing directory; the non-directory {}: {} cannot replace it",
                            host.display(),
                            entry.subsystem,
                            entry.path
                        ));
                    } else if metadata.is_file() {
                        // A regular-file output may replace an existing
                        // regular file. A symbolic link output may not:
                        // on Unix it would delete the file and change
                        // the filesystem object type, which is not an
                        // "overwrite". (On Windows the entry projects
                        // to a regular breadcrumb file instead.)
                        if cfg!(unix) && entry.file_type == FileType::SymbolicLink {
                            conflicts.push(format!(
                                "{} is an existing regular file; the symbolic link {}: {} cannot replace it",
                                host.display(),
                                entry.subsystem,
                                entry.path
                            ));
                        } else {
                            existing_regular += 1;
                        }
                    } else {
                        conflicts.push(format!(
                            "{} is an existing special file; refusing to replace it",
                            host.display()
                        ));
                    }
                }
            }
        }
    }
    if !conflicts.is_empty() {
        let mut message = String::from("extraction conflicts with existing filesystem objects\n");
        for conflict in conflicts.iter().take(5) {
            let _ = write!(message, "\n{conflict}");
        }
        if conflicts.len() > 5 {
            let _ = write!(message, "\n... and {} more.", conflicts.len() - 5);
        }
        message.push_str("\n\nNo files were written.");
        return Err(plan_error(message));
    }
    if !overwrites.is_empty() {
        let mut message = String::from("extraction would overwrite existing files\n");
        for path in overwrites.iter().take(5) {
            let _ = write!(message, "\n{}", path.display());
        }
        if overwrites.len() > 5 {
            let _ = write!(message, "\n... and {} more.", overwrites.len() - 5);
        }
        message.push_str("\n\nNo files were written.");
        return Err(plan_error(message));
    }
    Ok(existing_regular)
}

/// Wraps a refusal reason in the planner's error type.
fn plan_error(message: impl Into<String>) -> Error {
    Error::ExtractionPlan {
        message: message.into(),
    }
}
