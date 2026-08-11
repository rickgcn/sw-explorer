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
//! the source of all three hierarchy levels; a level whose expressions
//! are not decoded (`None`, which only happens without a descriptor
//! record) is *unknown*, not "no restriction", and is therefore not used
//! to exclude anything. Within one subsystem, several entries may share
//! a path: a matching mach-specific entry wins over the mach-less
//! fallback.
//!
//! Evaluation is fail-safe: a level whose expressions could not be
//! parsed reports [`Applicability::Unknown`], and the affected entries
//! are reported as unresolved conflicts instead of being silently
//! selected (fail-open) or dropped (fail-closed data loss).
//!
//! Ambiguities are reported through [`SelectionConflict`] *and* the
//! candidates stay in `selected`, so nothing silently disappears; it is
//! the caller's job to resolve or reject conflicts before extraction.
//! Entries whose `mach` attribute could not be parsed are the exception:
//! their applicability is unknown, so they are reported as conflicts but
//! never selected.
//!
//! Every entry the selection reports carries its canonical
//! [`LocatedEntry`] identity: the owning product is known from the
//! traversal itself, never guessed from the record's subsystem name.
use crate::descriptor::model::{Applicability, HardwareRestrictions, Subsystem};
use crate::distribution::{EntryKey, LocatedEntry, Product};
use crate::idb::{Entry, EntryId};
use crate::image::Image;
use crate::mach::eval::HardwareProfile;
use crate::path::IrixPath;

/// Several entries claim the same path and the selection rules cannot
/// pick between them, or the applicability of the candidates could not
/// be determined.
#[derive(Debug)]
pub struct SelectionConflict<'a> {
    /// The contested path.
    pub path: IrixPath,
    /// The competing candidates, with their owning-product identity.
    pub candidates: Vec<LocatedEntry<'a>>,
}

/// The result of selecting the entries applicable to a target.
#[derive(Debug, Default)]
pub struct EntrySelection<'a> {
    /// Entries that would be installed, in IDB order, with their
    /// owning-product identity. Conflict candidates are included; check
    /// `conflicts` before extracting.
    pub selected: Vec<LocatedEntry<'a>>,
    /// Paths with ambiguous or unresolvable candidates.
    pub conflicts: Vec<SelectionConflict<'a>>,
}

/// Selects the applicable entries of one product for a target.
///
/// `product_index` is the product's position in
/// [`crate::distribution::Distribution::products`]; it becomes the
/// owning-product half of every reported [`EntryKey`].
pub(crate) fn select_product<'a>(
    product_index: usize,
    product: &'a Product,
    profile: &HardwareProfile,
) -> EntrySelection<'a> {
    let mut selection = EntrySelection::default();
    match applicability(&product.mach, profile) {
        Applicability::NoMatch => return selection,
        Applicability::Unknown => {
            report_unresolved(locate_all(product_index, product), &mut selection);
            return selection;
        }
        Applicability::Match => {}
    }

    for image in &product.images {
        match applicability(&image.mach, profile) {
            Applicability::NoMatch => continue,
            Applicability::Unknown => {
                report_unresolved(image_entries(product_index, product, image), &mut selection);
                continue;
            }
            Applicability::Match => {}
        }
        for subsystem in &image.subsystems {
            match applicability(&subsystem.mach, profile) {
                Applicability::NoMatch => continue,
                Applicability::Unknown => {
                    report_unresolved(
                        subsystem_entries(product_index, product, subsystem),
                        &mut selection,
                    );
                    continue;
                }
                Applicability::Match => {}
            }
            select_subsystem_entries(
                product_index,
                product,
                &subsystem.entry_ids,
                profile,
                &mut selection,
            );
        }
    }
    selection
}

/// Pairs one entry of a product with its canonical key.
fn locate(product_index: usize, entry: &Entry) -> LocatedEntry<'_> {
    LocatedEntry {
        key: EntryKey {
            product_index,
            entry_id: entry.id,
        },
        entry,
    }
}

/// Every entry of a product, in IDB order.
fn locate_all<'a>(
    product_index: usize,
    product: &'a Product,
) -> impl Iterator<Item = LocatedEntry<'a>> {
    product
        .entries
        .iter()
        .map(move |entry| locate(product_index, entry))
}

/// Evaluates a hierarchy level's hardware restrictions for the target.
///
/// `None` means the level has no descriptor record (its applicability
/// is unknown to us); the established policy is to not exclude anything
/// on that basis.
fn applicability(mach: &Option<HardwareRestrictions>, profile: &HardwareProfile) -> Applicability {
    match mach {
        Some(restrictions) => restrictions.evaluate(profile),
        None => Applicability::Match,
    }
}

/// The entries of one image.
fn image_entries<'a>(
    product_index: usize,
    product: &'a Product,
    image: &'a Image,
) -> impl Iterator<Item = LocatedEntry<'a>> {
    image
        .subsystems
        .iter()
        .flat_map(move |subsystem| subsystem_entries(product_index, product, subsystem))
}

/// The entries of one subsystem.
fn subsystem_entries<'a>(
    product_index: usize,
    product: &'a Product,
    subsystem: &'a Subsystem,
) -> impl Iterator<Item = LocatedEntry<'a>> {
    subsystem
        .entry_ids
        .iter()
        .filter_map(move |id| product.entry(*id))
        .map(move |entry| locate(product_index, entry))
}

/// Reports entries whose applicability cannot be determined as
/// conflicts, grouped by path.
fn report_unresolved<'a>(
    entries: impl Iterator<Item = LocatedEntry<'a>>,
    selection: &mut EntrySelection<'a>,
) {
    let mut groups: Vec<(&IrixPath, Vec<LocatedEntry>)> = Vec::new();
    for located in entries {
        match groups
            .iter_mut()
            .find(|(path, _)| *path == &located.entry.path)
        {
            Some((_, entries)) => entries.push(located),
            None => groups.push((&located.entry.path, vec![located])),
        }
    }
    for (path, candidates) in groups {
        selection.conflicts.push(SelectionConflict {
            path: path.clone(),
            candidates,
        });
    }
}

fn select_subsystem_entries<'a>(
    product_index: usize,
    product: &'a Product,
    entry_ids: &[EntryId],
    profile: &HardwareProfile,
    selection: &mut EntrySelection<'a>,
) {
    // Group by path, keeping groups in first-appearance (IDB) order.
    let mut groups: Vec<(&IrixPath, Vec<LocatedEntry>)> = Vec::new();
    for id in entry_ids {
        if let Some(entry) = product.entries.get(id.0) {
            let located = locate(product_index, entry);
            match groups.iter_mut().find(|(path, _)| *path == &entry.path) {
                Some((_, entries)) => entries.push(located),
                None => groups.push((&entry.path, vec![located])),
            }
        }
    }

    for (path, entries) in groups {
        let mut specific: Vec<LocatedEntry> = Vec::new();
        let mut fallback: Vec<LocatedEntry> = Vec::new();
        let mut unresolvable: Vec<LocatedEntry> = Vec::new();
        for located in entries {
            if located.entry.has_unparsed_mach() {
                // Never treat an unparseable hardware restriction as a
                // mach-less fallback: applicability is unknown.
                unresolvable.push(located);
                continue;
            }
            let mach: Vec<&crate::mach::HardwareExpr> = located.entry.mach().collect();
            if mach.is_empty() {
                fallback.push(located);
            } else if mach.iter().any(|e| e.evaluate(profile)) {
                specific.push(located);
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
