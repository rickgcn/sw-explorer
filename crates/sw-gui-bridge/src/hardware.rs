//! Hardware-profile selection support: extraction of hardware
//! attribute candidates from the loaded distribution and conversion of
//! a `sw-core` entry selection into the bridge DTOs.
//!
//! Candidates come exclusively from the MACH expressions the current
//! distribution actually carries — product, image and subsystem
//! descriptor restrictions plus entry `mach(...)` attributes. The
//! parsed expression trees are walked; payloads that could not be
//! parsed are never guessed at with textual tricks, so an unresolved
//! expression simply contributes no candidates. There is no built-in
//! machine or board database: the distribution metadata is the only
//! authority.
//!
//! Selection itself is computed by `sw-core` alone; this module only
//! translates the resulting entry references into stable backend keys.
use crate::bridge::ffi;
use std::collections::{HashMap, HashSet};
use sw_core::distribution::Distribution;
use sw_core::idb::Entry;
use sw_core::mach::{HardwareExpr, MachAttribute, MachExpr};
use sw_core::selection::EntrySelection;

/// The canonical name under which a hardware attribute is reported.
/// Unknown attributes keep their verbatim name; nothing is dropped.
fn attribute_name(attribute: &MachAttribute) -> String {
    match attribute {
        MachAttribute::CpuBoard => "CPUBOARD".to_string(),
        MachAttribute::CpuArch => "CPUARCH".to_string(),
        MachAttribute::GfxBoard => "GFXBOARD".to_string(),
        MachAttribute::Subgr => "SUBGR".to_string(),
        MachAttribute::Video => "VIDEO".to_string(),
        MachAttribute::Mode => "MODE".to_string(),
        MachAttribute::TargetOs => "TARGOS".to_string(),
        MachAttribute::DistributionOs => "DISTOS".to_string(),
        MachAttribute::Unknown(name) => name.clone(),
    }
}

/// Accumulates attribute/value pairs in first-appearance order,
/// deduplicating exact repeats.
#[derive(Default)]
struct CandidateCollector {
    sets: Vec<ffi::HardwareCandidateSet>,
    attribute_index: HashMap<String, usize>,
    seen: HashSet<(String, String)>,
}

impl CandidateCollector {
    /// Records one comparison's right-hand side as a candidate value
    /// of its attribute.
    fn record(&mut self, attribute: &MachAttribute, value: &str) {
        let attribute = attribute_name(attribute);
        if !self.seen.insert((attribute.clone(), value.to_string())) {
            return;
        }
        let index = *self
            .attribute_index
            .entry(attribute.clone())
            .or_insert_with_key(|attribute| {
                self.sets.push(ffi::HardwareCandidateSet {
                    attribute: attribute.clone(),
                    values: Vec::new(),
                });
                self.sets.len() - 1
            });
        self.sets[index].values.push(value.to_string());
    }

    /// Walks one expression tree, recording every comparison value,
    /// whatever operator or nesting it appears under: any value written
    /// on the media is a value the target machine might report.
    fn walk(&mut self, expr: &MachExpr) {
        match expr {
            MachExpr::Compare {
                attribute, value, ..
            } => self.record(attribute, value),
            MachExpr::And(operands) | MachExpr::Or(operands) => {
                for operand in operands {
                    self.walk(operand);
                }
            }
            MachExpr::Not(operand) => self.walk(operand),
        }
    }

    /// Walks one parsed expression.
    fn expression(&mut self, expression: &HardwareExpr) {
        self.walk(&expression.expr);
    }
}

/// The hardware attribute candidates of the loaded distribution:
/// every attribute/value pair occurring in a successfully parsed MACH
/// expression, grouped by attribute, in first-appearance order.
///
/// Sources per product, in order: the product restrictions, then each
/// image with its subsystems, then the entry `mach(...)` attributes in
/// IDB order. Unparsed expression payloads contribute nothing.
pub(crate) fn hardware_candidates(distribution: &Distribution) -> Vec<ffi::HardwareCandidateSet> {
    let mut collector = CandidateCollector::default();
    for product in distribution.products() {
        if let Some(mach) = &product.mach {
            for expression in &mach.expressions {
                collector.expression(expression);
            }
        }
        for image in &product.images {
            if let Some(mach) = &image.mach {
                for expression in &mach.expressions {
                    collector.expression(expression);
                }
            }
            for subsystem in &image.subsystems {
                if let Some(mach) = &subsystem.mach {
                    for expression in &mach.expressions {
                        collector.expression(expression);
                    }
                }
            }
        }
        for entry in &product.entries {
            for expression in entry.mach() {
                collector.expression(expression);
            }
        }
    }
    collector.sets
}

/// Converts a `sw-core` entry selection into the bridge DTO, resolving
/// every entry reference to the object id of the product whose entry
/// list actually holds it through `owners`.
///
/// Conflicts are passed through exactly as the core reports them: no
/// candidate is picked, dropped or merged, and conflict candidates
/// that are also selected appear in both lists.
pub(crate) fn selection_snapshot(
    selection: &EntrySelection,
    owners: &HashMap<*const Entry, u64>,
) -> Result<ffi::SelectionSnapshot, String> {
    let key_of = |entry: &Entry| -> Result<ffi::SelectionEntryKey, String> {
        let product_id = owners
            .get(&std::ptr::from_ref(entry))
            .ok_or_else(|| "selected entry has no owning product".to_string())?;
        Ok(ffi::SelectionEntryKey {
            product_id: *product_id,
            entry_id: entry.id.0 as u64,
        })
    };

    let mut selected = Vec::with_capacity(selection.selected.len());
    for entry in &selection.selected {
        selected.push(key_of(entry)?);
    }
    let mut conflicts = Vec::with_capacity(selection.conflicts.len());
    for conflict in &selection.conflicts {
        let mut candidates = Vec::with_capacity(conflict.candidates.len());
        for entry in &conflict.candidates {
            candidates.push(key_of(entry)?);
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
