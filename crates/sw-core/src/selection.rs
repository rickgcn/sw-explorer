//! Hardware-based entry selection, approximating what `inst` installs.
//!
//! An entry applies to a target only when the hardware expressions of
//! every level above it match:
//!
//! ```text
//! Product MACH && Image MACH && Subsystem MACH && Entry MACH
//! ```
//!
//! (multiple expressions on the same level are OR-ed). The descriptor is
//! the source of product- and image-level expressions; a level whose
//! expressions are not decoded (`None`) is *unknown*, not "no
//! restriction", and is therefore not used to exclude anything. Within
//! one subsystem, several entries may share a path: a matching
//! mach-specific entry wins over the mach-less fallback.
//!
//! Ambiguities are reported through [`SelectionConflict`] *and* the
//! candidates stay in `selected`, so nothing silently disappears; it is
//! the caller's job to resolve or reject conflicts before extraction.
//! Entries whose `mach` attribute could not be parsed are the exception:
//! their applicability is unknown, so they are reported as conflicts but
//! never selected.
use crate::distribution::Product;
use crate::idb::{Entry, EntryId};
use crate::mach::eval::{HardwareProfile, matches_any};
use crate::path::IrixPath;

/// Several entries claim the same path and the selection rules cannot
/// pick between them.
#[derive(Debug)]
pub struct SelectionConflict<'a> {
    /// The contested path.
    pub path: IrixPath,
    /// The competing candidates.
    pub candidates: Vec<&'a Entry>,
}

/// The result of selecting the entries applicable to a target.
#[derive(Debug, Default)]
pub struct EntrySelection<'a> {
    /// Entries that would be installed, in IDB order. Conflict candidates
    /// are included; check `conflicts` before extracting.
    pub selected: Vec<&'a Entry>,
    /// Paths with ambiguous or unresolvable candidates.
    pub conflicts: Vec<SelectionConflict<'a>>,
}

/// Selects the applicable entries of one product for a target.
pub fn select_product<'a>(product: &'a Product, profile: &HardwareProfile) -> EntrySelection<'a> {
    let mut selection = EntrySelection::default();
    if !mach_matches(&product.mach, profile) {
        return selection;
    }

    for image in &product.images {
        if !mach_matches(&image.mach, profile) {
            continue;
        }
        for subsystem in &image.subsystems {
            if !mach_matches(&subsystem.mach, profile) {
                continue;
            }
            select_subsystem_entries(product, &subsystem.entry_ids, profile, &mut selection);
        }
    }
    selection
}

/// Whether a hierarchy level's hardware expressions match the target.
///
/// `None` means the expressions are not decoded (unknown), which never
/// excludes anything; `Some` applies the OR-ed expressions, with an
/// empty list matching everything.
fn mach_matches(mach: &Option<Vec<crate::mach::HardwareExpr>>, profile: &HardwareProfile) -> bool {
    match mach {
        Some(expressions) => matches_any(expressions, profile),
        None => true,
    }
}

fn select_subsystem_entries<'a>(
    product: &'a Product,
    entry_ids: &[EntryId],
    profile: &HardwareProfile,
    selection: &mut EntrySelection<'a>,
) {
    // Group by path, keeping groups in first-appearance (IDB) order.
    let mut groups: Vec<(&IrixPath, Vec<&Entry>)> = Vec::new();
    for id in entry_ids {
        if let Some(entry) = product.entries.get(id.0) {
            match groups.iter_mut().find(|(path, _)| *path == &entry.path) {
                Some((_, entries)) => entries.push(entry),
                None => groups.push((&entry.path, vec![entry])),
            }
        }
    }

    for (path, entries) in groups {
        let mut specific: Vec<&Entry> = Vec::new();
        let mut fallback: Vec<&Entry> = Vec::new();
        let mut unresolvable: Vec<&Entry> = Vec::new();
        for entry in entries {
            if entry.has_unparsed_mach() {
                // Never treat an unparseable hardware restriction as a
                // mach-less fallback: applicability is unknown.
                unresolvable.push(entry);
                continue;
            }
            let mach: Vec<&crate::mach::HardwareExpr> = entry.mach().collect();
            if mach.is_empty() {
                fallback.push(entry);
            } else if mach.iter().any(|e| e.evaluate(profile)) {
                specific.push(entry);
            }
        }

        if !unresolvable.is_empty() {
            selection.conflicts.push(SelectionConflict {
                path: path.clone(),
                candidates: unresolvable,
            });
        }
        match specific.len() {
            0 => {
                if fallback.len() > 1 {
                    selection.conflicts.push(SelectionConflict {
                        path: path.clone(),
                        candidates: fallback.clone(),
                    });
                }
                selection.selected.extend(fallback);
            }
            1 => selection.selected.push(specific[0]),
            _ => {
                selection.conflicts.push(SelectionConflict {
                    path: path.clone(),
                    candidates: specific.clone(),
                });
                selection.selected.extend(specific);
            }
        }
    }
}
