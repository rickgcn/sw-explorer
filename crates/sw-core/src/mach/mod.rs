//! Hardware applicability expressions (`mach`).
//!
//! SGI defines two hardware expression syntaxes that must not be mixed:
//!
//! *Legacy list* syntax, a whitespace-separated list of `ATTRIBUTE=VALUE`
//! pairs. Values of the same attribute are OR-ed, different attributes are
//! AND-ed:
//!
//! ```text
//! CPUBOARD=IP12 GFXBOARD=LIGHT GFXBOARD=EXPRESS
//! ```
//!
//! means `CPUBOARD=IP12 AND (GFXBOARD=LIGHT OR GFXBOARD=EXPRESS)`.
//!
//! *Boolean* syntax, a full expression language:
//!
//! ```text
//! CPUBOARD=IP22 && (GFXBOARD=EXPRESS || GFXBOARD=NEWPRESS)
//! ```
//!
//! supporting `&&`, `||`, `!`, `=`, `!=`, `<`, `<=`, `>`, `>=` and
//! parentheses.
//!
//! In both syntaxes a bare value with no attribute is shorthand for a
//! `CPUBOARD` comparison, per `gendist(1M)`: `IP22` means
//! `CPUBOARD=IP22`, and `IP22 || IP26` means
//! `CPUBOARD=IP22 || CPUBOARD=IP26`.

pub mod candidates;
pub mod eval;
pub(crate) mod lexer;
pub mod parser;

/// Which of the two hardware expression syntaxes an expression uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareSyntax {
    /// Whitespace-separated `ATTRIBUTE=VALUE` list.
    LegacyList,
    /// Boolean expression with `&&`, `||`, `!` and parentheses.
    Boolean,
}

/// A parsed hardware expression, keeping the original text for display.
#[derive(Debug, Clone, PartialEq)]
pub struct HardwareExpr {
    /// The original expression text.
    pub raw: String,
    /// Which syntax the expression was parsed as.
    pub syntax: HardwareSyntax,
    /// The parsed expression tree.
    pub expr: MachExpr,
}

/// Hardware expression AST.
#[derive(Debug, Clone, PartialEq)]
pub enum MachExpr {
    /// A single attribute comparison, e.g. `CPUBOARD=IP22`.
    Compare {
        /// The hardware attribute being compared.
        attribute: MachAttribute,
        /// The comparison operator.
        op: CompareOp,
        /// The right-hand side value, e.g. `IP22`.
        value: String,
    },
    /// All sub-expressions must match.
    And(Vec<MachExpr>),
    /// At least one sub-expression must match.
    Or(Vec<MachExpr>),
    /// The sub-expression must not match.
    Not(Box<MachExpr>),
}

/// Hardware attributes defined by SGI for `mach` expressions.
///
/// `Unknown` preserves attributes not (yet) known to this library, so that
/// no information is lost when the format evolves.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MachAttribute {
    /// CPU board, e.g. `IP22`.
    CpuBoard,
    /// CPU architecture, e.g. `R4000`.
    CpuArch,
    /// Graphics board, e.g. `EXPRESS`.
    GfxBoard,
    /// Graphics sub-group.
    Subgr,
    /// Video hardware.
    Video,
    /// Addressing mode, e.g. `32bit` / `64bit`.
    Mode,
    /// Target operating system.
    TargetOs,
    /// Distribution operating system.
    DistributionOs,
    /// Any other attribute name, preserved verbatim.
    Unknown(String),
}

impl MachAttribute {
    /// Maps an attribute name to its known variant, preserving unknown
    /// names.
    pub fn from_name(name: &str) -> Self {
        match name {
            "CPUBOARD" => MachAttribute::CpuBoard,
            "CPUARCH" => MachAttribute::CpuArch,
            "GFXBOARD" => MachAttribute::GfxBoard,
            "SUBGR" => MachAttribute::Subgr,
            "VIDEO" => MachAttribute::Video,
            "MODE" => MachAttribute::Mode,
            "TARGOS" => MachAttribute::TargetOs,
            "DISTOS" => MachAttribute::DistributionOs,
            other => MachAttribute::Unknown(other.to_string()),
        }
    }
}

/// Comparison operators of the boolean hardware syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
}
