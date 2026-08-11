//! Hardware-selection DTO conversion: translates a `sw-core` entry
//! selection into the bridge DTOs.
//!
//! All domain semantics — MACH candidate discovery, hierarchy
//! restrictions, mach-less fallback, duplicate paths, unresolved
//! expressions and conflicts — live in `sw-core`; this module only maps
//! the resulting located entries to stable backend object ids.
use crate::bridge::ffi;
use sw_core::distribution::LocatedEntry;
use sw_core::selection::EntrySelection;

/// Converts a `sw-core` entry selection into the bridge DTO, resolving
/// every entry's canonical key to the object id of the product that
/// actually owns it through `product_object_ids` (indexed by
/// `EntryKey::product_index`).
///
/// Conflicts are passed through exactly as the core reports them: no
/// candidate is picked, dropped or merged, and conflict candidates
/// that are also selected appear in both lists.
pub(crate) fn selection_snapshot(
    selection: &EntrySelection,
    product_object_ids: &[u64],
) -> Result<ffi::SelectionSnapshot, String> {
    let key_of = |located: &LocatedEntry| -> Result<ffi::SelectionEntryKey, String> {
        let product_id = product_object_ids
            .get(located.key.product_index)
            .ok_or_else(|| "selected entry has no owning product".to_string())?;
        Ok(ffi::SelectionEntryKey {
            product_id: *product_id,
            entry_id: located.key.entry_id.0 as u64,
        })
    };

    let mut selected = Vec::with_capacity(selection.selected.len());
    for located in &selection.selected {
        selected.push(key_of(located)?);
    }
    let mut conflicts = Vec::with_capacity(selection.conflicts.len());
    for conflict in &selection.conflicts {
        let mut candidates = Vec::with_capacity(conflict.candidates.len());
        for located in &conflict.candidates {
            candidates.push(key_of(located)?);
        }
        conflicts.push(ffi::SelectionConflictDetail {
            path: conflict.path.to_string(),
            candidates,
        });
    }
    Ok(ffi::SelectionSnapshot {
        selected,
        conflicts,
    })
}
