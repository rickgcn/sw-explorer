//! Discovery of hardware attribute candidates in a distribution.
//!
//! Candidates come exclusively from the MACH expressions the
//! distribution actually carries — product, image and subsystem
//! descriptor restrictions plus entry `mach(...)` attributes. The parsed
//! expression trees are walked; payloads that could not be parsed are
//! never guessed at with textual tricks, so an unresolved expression
//! simply contributes no candidates. There is no built-in machine or
//! board database: the distribution metadata is the only authority.
use crate::distribution::Distribution;
use crate::mach::{HardwareExpr, MachAttribute, MachExpr};
use std::collections::{HashMap, HashSet};

/// The candidate values of one hardware attribute: every value the
/// distribution's parsed MACH expressions compare the attribute
/// against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareCandidateSet {
    /// The canonical attribute name, e.g. `CPUBOARD`; unknown
    /// attributes keep their verbatim name.
    pub attribute: String,
    /// Candidate values in first-appearance order, exact duplicates
    /// removed. An empty value is kept: the media carry restrictions
    /// like `GFXBOARD=` for headless boards.
    pub values: Vec<String>,
}

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
    sets: Vec<HardwareCandidateSet>,
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
                self.sets.push(HardwareCandidateSet {
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

/// The hardware attribute candidates of a distribution: every
/// attribute/value pair occurring in a successfully parsed MACH
/// expression, grouped by attribute, in first-appearance order.
///
/// Sources per product, in order: the product restrictions, then each
/// image with its subsystems, then the entry `mach(...)` attributes in
/// IDB order. Unparsed expression payloads contribute nothing.
pub fn hardware_candidates(distribution: &Distribution) -> Vec<HardwareCandidateSet> {
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
