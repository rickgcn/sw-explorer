//! Command-line interface definition (argument parsing only).

use clap::{ArgGroup, Args, Parser, Subcommand};
use std::path::PathBuf;
use sw_core::names::{ImageName, ProductName, SubsystemName};
use sw_core::path::IrixPath;

/// Browse, search and extract SGI IRIX software distributions.
#[derive(Debug, Parser)]
#[command(name = "sw", version, about)]
pub struct Cli {
    /// Path to the distribution directory.
    #[arg(
        short = 'd',
        long = "dist",
        global = true,
        default_value = ".",
        value_name = "PATH"
    )]
    pub dist: PathBuf,

    #[command(subcommand)]
    pub command: Command,
}

/// The available commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// List the products of the distribution.
    Products,
    /// Show the product / image / subsystem tree of one product.
    Tree(TreeArgs),
    /// Show the details of a product, image or subsystem.
    Show(ShowArgs),
    /// Search file entries by path.
    Find(FindArgs),
    /// Show which entries a hardware profile selects.
    Select(SelectArgs),
    /// Extract files to the host filesystem.
    Extract(ExtractArgs),
}

/// Hardware profile values shared by the commands that need one.
#[derive(Debug, Clone, Default, Args)]
pub struct MachArgs {
    /// Hardware attribute value, e.g. `GFXBOARD=EXPRESS`; a bare value
    /// such as `IP22` is shorthand for `CPUBOARD=IP22`. Repeatable, and
    /// an attribute may carry several values.
    #[arg(long = "mach", value_name = "VALUE", value_parser = parse_mach_value)]
    pub mach: Vec<String>,
}

/// Arguments of `sw tree`.
#[derive(Debug, Args)]
pub struct TreeArgs {
    /// Product name, e.g. `eoe`.
    pub product: String,
}

/// Arguments of `sw show`.
#[derive(Debug, Args)]
pub struct ShowArgs {
    /// `product`, `product.image` or `product.image.subsystem`.
    #[arg(value_parser = parse_target_name)]
    pub name: String,
}

/// Arguments of `sw find`.
#[derive(Debug, Args)]
pub struct FindArgs {
    /// Path query: a plain string matches any path containing it; `*`
    /// and `?` make it a wildcard pattern.
    pub pattern: String,

    #[command(flatten)]
    pub mach: MachArgs,
}

/// Arguments of `sw select`.
#[derive(Debug, Args)]
pub struct SelectArgs {
    #[command(flatten)]
    pub mach: MachArgs,

    /// Also list every selected entry path.
    #[arg(long)]
    pub list: bool,
}

/// Arguments of `sw extract`.
#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("scope")
        .args(["path", "subsystem", "image", "product"])
        .required(true)
        .multiple(false)
))]
pub struct ExtractArgs {
    /// Extract entries whose path matches PATTERN (same query syntax as
    /// `sw find`).
    #[arg(long, value_name = "PATTERN")]
    pub path: Option<String>,

    /// Extract one subsystem, e.g. `eoe.sw.unix`.
    #[arg(long, value_name = "NAME", value_parser = parse_subsystem_name)]
    pub subsystem: Option<String>,

    /// Extract one image, e.g. `eoe.sw`.
    #[arg(long, value_name = "NAME", value_parser = parse_image_name)]
    pub image: Option<String>,

    /// Extract one product, e.g. `eoe`.
    #[arg(long, value_name = "NAME", value_parser = parse_product_name)]
    pub product: Option<String>,

    #[command(flatten)]
    pub mach: MachArgs,

    /// Output directory.
    #[arg(short = 'o', long = "output", value_name = "DIR")]
    pub output: PathBuf,

    /// Write every entry directly into the output directory.
    #[arg(long, conflicts_with = "relative_to")]
    pub flat: bool,

    /// Strip the given IRIX path prefix from extracted paths.
    #[arg(long, value_name = "PATH", value_parser = parse_irix_path)]
    pub relative_to: Option<IrixPath>,

    /// Write payloads exactly as stored, without decoding `.Z` data.
    #[arg(long)]
    pub raw: bool,

    /// Additionally keep the stored (compressed) bytes as `<name>.Z`.
    #[arg(long)]
    pub keep_stored: bool,

    /// Stop at the first extraction error.
    #[arg(long)]
    pub fail_fast: bool,
}

/// Validates a `--mach` value: `VALUE` or `ATTRIBUTE=VALUE`. The
/// attribute name is never empty; the value may be (`GFXBOARD=`
/// restricts a record to headless boards on real media).
fn parse_mach_value(value: &str) -> Result<String, String> {
    match value.split_once('=') {
        Some((attribute, _)) if !attribute.is_empty() => Ok(value.to_string()),
        None if !value.is_empty() => Ok(value.to_string()),
        _ => Err(format!(
            "invalid hardware value {value:?}: expected VALUE or ATTRIBUTE=VALUE"
        )),
    }
}

/// Validates a `show` target: one to three non-empty dotted segments.
fn parse_target_name(value: &str) -> Result<String, String> {
    let segments: Vec<&str> = value.split('.').collect();
    if (1..=3).contains(&segments.len()) && segments.iter().all(|s| !s.is_empty()) {
        Ok(value.to_string())
    } else {
        Err(format!(
            "expected a product, image or subsystem name \
             (1 to 3 dot-separated segments), got {value:?}"
        ))
    }
}

fn parse_product_name(value: &str) -> Result<String, String> {
    ProductName::new(value)
        .map(|_| value.to_string())
        .map_err(|error| error.to_string())
}

fn parse_image_name(value: &str) -> Result<String, String> {
    let segments: Vec<&str> = value.split('.').collect();
    if segments.len() != 2 {
        return Err(format!(
            "expected an image name like `eoe.sw`, got {value:?}"
        ));
    }
    let product = ProductName::new(segments[0]).map_err(|error| error.to_string())?;
    ImageName::from_parts(product, segments[1].to_string())
        .map(|_| value.to_string())
        .map_err(|error| error.to_string())
}

fn parse_subsystem_name(value: &str) -> Result<String, String> {
    SubsystemName::parse(value)
        .map(|_| value.to_string())
        .map_err(|error| error.to_string())
}

fn parse_irix_path(value: &str) -> Result<IrixPath, String> {
    IrixPath::new(value).map_err(|error| error.to_string())
}
