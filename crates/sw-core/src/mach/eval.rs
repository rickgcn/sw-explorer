//! Evaluation of hardware expressions against a [`HardwareProfile`].

use super::{CompareOp, HardwareExpr, MachAttribute, MachExpr};
use std::collections::{BTreeMap, BTreeSet};

/// A simulated installation target.
///
/// One attribute can carry several values: IRIX itself may report e.g.
/// both `CPUARCH=MIPS2` and `CPUARCH=R4000` for a single machine.
///
/// The library deliberately does not ship a "machine model to attributes"
/// database; the caller describes the target and the distribution data
/// alone decides which files apply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HardwareProfile {
    /// Attribute values keyed by hardware attribute.
    pub values: BTreeMap<MachAttribute, BTreeSet<String>>,
}

impl HardwareProfile {
    /// Starts building a profile.
    pub fn builder() -> HardwareProfileBuilder {
        HardwareProfileBuilder {
            values: BTreeMap::new(),
        }
    }

    /// Looks up the values of an attribute.
    pub fn get(&self, attribute: &MachAttribute) -> Option<&BTreeSet<String>> {
        self.values.get(attribute)
    }
}

/// Builder for [`HardwareProfile`].
#[derive(Debug, Default)]
pub struct HardwareProfileBuilder {
    values: BTreeMap<MachAttribute, BTreeSet<String>>,
}

impl HardwareProfileBuilder {
    /// Sets an attribute to a single value, replacing previous values.
    pub fn set(mut self, attribute: &str, value: &str) -> Self {
        self.values.insert(
            MachAttribute::from_name(attribute),
            BTreeSet::from([value.to_string()]),
        );
        self
    }

    /// Adds a value to an attribute, keeping previous values.
    pub fn add(mut self, attribute: &str, value: &str) -> Self {
        self.values
            .entry(MachAttribute::from_name(attribute))
            .or_default()
            .insert(value.to_string());
        self
    }

    /// Finishes the profile.
    pub fn build(self) -> HardwareProfile {
        HardwareProfile {
            values: self.values,
        }
    }
}

impl HardwareExpr {
    /// Evaluates this expression against a hardware profile.
    ///
    /// Equality holds when *any* value of the attribute matches, inequality
    /// when *no* value matches, and ordering operators when any value
    /// satisfies them. A comparison whose attribute is absent from the
    /// profile evaluates to `false`, for every operator.
    pub fn evaluate(&self, profile: &HardwareProfile) -> bool {
        evaluate(&self.expr, profile)
    }
}

/// Evaluates an expression tree against a hardware profile.
pub fn evaluate(expr: &MachExpr, profile: &HardwareProfile) -> bool {
    match expr {
        MachExpr::Compare {
            attribute,
            op,
            value,
        } => match profile.get(attribute) {
            Some(actual) => compare(actual, *op, value),
            None => false,
        },
        MachExpr::And(operands) => operands.iter().all(|e| evaluate(e, profile)),
        MachExpr::Or(operands) => operands.iter().any(|e| evaluate(e, profile)),
        MachExpr::Not(operand) => !evaluate(operand, profile),
    }
}

/// Whether a node carrying several `mach` expressions applies to a target.
///
/// Per SGI semantics, multiple expressions on the same node are OR-ed. A
/// node without any expression applies to every target.
pub fn matches_any(exprs: &[HardwareExpr], profile: &HardwareProfile) -> bool {
    exprs.is_empty() || exprs.iter().any(|e| e.evaluate(profile))
}

fn compare(actual: &BTreeSet<String>, op: CompareOp, expected: &str) -> bool {
    match op {
        CompareOp::Eq => actual.iter().any(|a| a == expected),
        CompareOp::Ne => actual.iter().all(|a| a != expected),
        CompareOp::Lt => actual.iter().any(|a| compare_one(a, expected).is_lt()),
        CompareOp::Le => actual.iter().any(|a| compare_one(a, expected).is_le()),
        CompareOp::Gt => actual.iter().any(|a| compare_one(a, expected).is_gt()),
        CompareOp::Ge => actual.iter().any(|a| compare_one(a, expected).is_ge()),
    }
}

/// Numeric comparison when both sides parse as integers, else lexicographic.
fn compare_one(actual: &str, expected: &str) -> std::cmp::Ordering {
    match (actual.parse::<i64>(), expected.parse::<i64>()) {
        (Ok(a), Ok(b)) => a.cmp(&b),
        _ => actual.cmp(expected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indigo2_extreme() -> HardwareProfile {
        HardwareProfile::builder()
            .set("CPUBOARD", "IP22")
            .set("CPUARCH", "R4400")
            .set("GFXBOARD", "EXPRESS")
            .set("MODE", "32bit")
            .build()
    }

    #[test]
    fn legacy_list_evaluation() {
        let profile = indigo2_extreme();
        let matching = HardwareExpr::parse("CPUBOARD=IP19 CPUBOARD=IP22").unwrap();
        let mismatching = HardwareExpr::parse("CPUBOARD=IP19 CPUBOARD=IP21").unwrap();
        assert!(matching.evaluate(&profile));
        assert!(!mismatching.evaluate(&profile));
    }

    #[test]
    fn boolean_evaluation() {
        let profile = indigo2_extreme();
        let expr = HardwareExpr::parse("CPUBOARD=IP22 && (GFXBOARD=EXPRESS || GFXBOARD=NEWPRESS)")
            .unwrap();
        assert!(expr.evaluate(&profile));
        let expr = HardwareExpr::parse("MODE=64bit && CPUBOARD!=IP26").unwrap();
        assert!(!expr.evaluate(&profile));
        let expr = HardwareExpr::parse("GFXBOARD!=SERVER").unwrap();
        assert!(expr.evaluate(&profile));
    }

    #[test]
    fn missing_attribute_is_false() {
        let profile = HardwareProfile::builder().build();
        let expr = HardwareExpr::parse("CPUBOARD=IP22").unwrap();
        assert!(!expr.evaluate(&profile));
    }

    #[test]
    fn matches_any_or_semantics() {
        let profile = indigo2_extreme();
        let exprs = vec![
            HardwareExpr::parse("CPUBOARD=IP19").unwrap(),
            HardwareExpr::parse("CPUBOARD=IP22").unwrap(),
        ];
        assert!(matches_any(&exprs, &profile));
        assert!(matches_any(&[], &profile));
    }

    #[test]
    fn multi_valued_attributes() {
        let profile = HardwareProfile::builder()
            .add("CPUARCH", "MIPS2")
            .add("CPUARCH", "R4000")
            .set("CPUBOARD", "IP22")
            .build();
        assert!(
            HardwareExpr::parse("CPUARCH=MIPS2")
                .unwrap()
                .evaluate(&profile)
        );
        assert!(
            HardwareExpr::parse("CPUARCH=R4000")
                .unwrap()
                .evaluate(&profile)
        );
        assert!(
            !HardwareExpr::parse("CPUARCH=MIPS1")
                .unwrap()
                .evaluate(&profile)
        );
        assert!(
            HardwareExpr::parse("CPUARCH!=MIPS1")
                .unwrap()
                .evaluate(&profile)
        );
        assert!(
            !HardwareExpr::parse("CPUARCH!=MIPS2")
                .unwrap()
                .evaluate(&profile)
        );
    }
}
