//! Extraction of selected entries to the host filesystem.
//!
//! All paths are [`crate::path::IrixPath`] values, which are normalized
//! and traversal-free by construction; extraction cannot escape the output
//! directory. On top of that, [`host_path`] validates every component
//! against the host filesystem's naming rules (which matter once IRIX
//! file names meet Windows).
use crate::error::{Error, Result};
use crate::idb::{Entry, FileType};
use crate::image::PayloadResolution;
use crate::image::reader::ImageReader;
use crate::path::IrixPath;
use std::path::{Path, PathBuf};

/// How entry paths are mapped below the output directory.
#[derive(Debug, Clone, Default)]
pub enum PathMode {
    /// Rebuild the full IRIX hierarchy below the output directory.
    #[default]
    Full,
    /// Write every entry directly into the output directory, using only
    /// its file name; later entries with the same name overwrite earlier
    /// ones.
    Flat,
    /// Strip the given prefix from entry paths. Entries outside the
    /// prefix are skipped.
    RelativeTo(IrixPath),
}

/// Whether compressed payloads are decoded during extraction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DecodeMode {
    /// Decode `.Z` payloads to their plain contents.
    #[default]
    Auto,
    /// Write payloads exactly as stored. Compressed payloads are written
    /// with `.Z` appended to the file name (e.g. `foo` becomes `foo.Z`);
    /// uncompressed payloads keep their plain name.
    Never,
}

/// How extraction treats output paths that already exist on the host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExistingOutputPolicy {
    /// Existing regular files may be overwritten by regular-file
    /// outputs. This is the historical behavior. Symbolic links still
    /// never replace an existing path: that would change the filesystem
    /// object type, not overwrite its contents.
    #[default]
    Allow,
    /// Refuse to overwrite or remove anything that already exists.
    /// Regular-file writes (decoded payloads, stored bytes, `.Z`
    /// sidecars and symlink breadcrumbs) use atomic no-clobber creation,
    /// so a target appearing after the caller's preflight still cannot
    /// be overwritten, and symbolic links are never replaced.
    Refuse,
}

/// Options controlling extraction.
///
/// The defaults continue the historical behavior: full paths, decoded
/// payloads, continuing past individual failures, and overwriting
/// existing regular files.
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    /// How entry paths are mapped below the output directory.
    pub path_mode: PathMode,
    /// Whether compressed payloads are decoded.
    pub decode: DecodeMode,
    /// With [`DecodeMode::Auto`], additionally write the original stored
    /// bytes of compressed payloads to `<name>.Z`.
    pub keep_stored: bool,
    /// Continue past individual failures instead of aborting.
    pub continue_on_error: bool,
    /// How output paths that already exist on the host are treated.
    pub existing_output: ExistingOutputPolicy,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        ExtractOptions {
            path_mode: PathMode::Full,
            decode: DecodeMode::Auto,
            keep_stored: false,
            continue_on_error: true,
            existing_output: ExistingOutputPolicy::Allow,
        }
    }
}

/// A single entry that failed to extract.
#[derive(Debug, Clone)]
pub struct ExtractFailure {
    /// Entry path.
    pub path: String,
    /// What went wrong.
    pub message: String,
}

/// A payload that was not found exactly at its expected offset.
///
/// Recoveries are honest successes — the bytes were located and written —
/// but the reader had to deviate from the computed layout to find them,
/// which callers must surface instead of silently accepting.
#[derive(Debug, Clone)]
pub struct ExtractRecovery {
    /// Entry path.
    pub path: String,
    /// Expected record offset, if the layout provided one.
    pub expected_record_offset: Option<u64>,
    /// Offset the record was actually found at.
    pub actual_record_offset: u64,
    /// How the record was located.
    pub resolution: PayloadResolution,
}

/// Outcome of an extraction run.
#[derive(Debug, Clone, Default)]
pub struct ExtractReport {
    /// Entries successfully written.
    pub extracted: usize,
    /// Entries deliberately not written (devices, FIFOs, ...).
    pub skipped: usize,
    /// Entries that failed.
    pub failures: Vec<ExtractFailure>,
    /// Payloads that needed recovery to be located.
    pub recoveries: Vec<ExtractRecovery>,
}

/// Extracts entries below `out_dir`.
///
/// Directories are created, regular files are written (decoding `.Z`
/// payloads according to [`ExtractOptions::decode`]), and symbolic links
/// are recreated on Unix. Permission bits are applied from the entry mode
/// on Unix. Existing output paths are treated according to
/// [`ExtractOptions::existing_output`]: with
/// [`ExistingOutputPolicy::Refuse`], regular-file writes fail atomically
/// instead of truncating an existing file, and symbolic links never
/// replace an existing path under either policy.
///
/// Callers that need the fail-safe gate (ambiguity, collisions, existing
/// output classification) should use [`crate::plan::extract_checked`]
/// instead of calling this directly.
pub fn extract(
    reader: &mut ImageReader<'_>,
    entries: &[&Entry],
    out_dir: &Path,
    options: &ExtractOptions,
) -> ExtractReport {
    let mut report = ExtractReport::default();

    for entry in entries {
        match extract_one(reader, entry, out_dir, options) {
            Ok(ExtractOutcome::Written(recovery)) => {
                report.extracted += 1;
                if let Some(recovery) = recovery {
                    report.recoveries.push(recovery);
                }
            }
            Ok(ExtractOutcome::Skipped) => report.skipped += 1,
            Err(error) => {
                report.failures.push(ExtractFailure {
                    path: entry.path.to_string(),
                    message: error.to_string(),
                });
                if !options.continue_on_error {
                    break;
                }
            }
        }
    }

    report
}

enum ExtractOutcome {
    Written(Option<ExtractRecovery>),
    Skipped,
}

/// Resolves an entry path against the path mode.
///
/// Returns `None` for entries the mode excludes (outside the
/// [`PathMode::RelativeTo`] prefix, or nameless under [`PathMode::Flat`]).
fn resolve_relative(entry: &Entry, mode: &PathMode) -> Option<IrixPath> {
    match mode {
        PathMode::Full => Some(entry.path.clone()),
        PathMode::Flat => entry
            .path
            .file_name()
            .and_then(|name| IrixPath::new(name).ok()),
        PathMode::RelativeTo(prefix) => entry.path.strip_prefix(prefix),
    }
}

/// Projects an entry to the host-relative paths [`extract`] would write
/// for it under `options`, without writing anything.
///
/// An empty list means [`extract`] would deliberately skip the entry:
/// excluded by the path mode, a device or FIFO, or a symbolic link
/// without a recorded target. Failures that are already known
/// statically — a host path that cannot be represented, a regular file
/// without a payload — are returned as errors, so a caller preflighting
/// with this projection fails before any bytes hit the disk instead of
/// silently dropping the entry.
///
/// # Errors
///
/// Returns [`Error::UnsafePath`] or [`Error::PayloadNotFound`] for
/// entries [`extract`] is statically known to fail on.
pub fn output_paths(entry: &Entry, options: &ExtractOptions) -> Result<Vec<PathBuf>> {
    let Some(relative) = resolve_relative(entry, &options.path_mode) else {
        return Ok(Vec::new());
    };
    let target = host_path(&relative)?;
    Ok(match entry.file_type {
        FileType::Directory => vec![target],
        FileType::Regular => {
            let stored_compressed = entry.compressed_size().is_some_and(|size| size > 0);
            if entry.payload.is_some() {
                match options.decode {
                    DecodeMode::Auto => {
                        let mut paths = vec![target];
                        if options.keep_stored && stored_compressed {
                            paths.push(stored_target(&paths[0]));
                        }
                        paths
                    }
                    DecodeMode::Never => {
                        if stored_compressed {
                            vec![stored_target(&target)]
                        } else {
                            vec![target]
                        }
                    }
                }
            } else if entry.size() == Some(0) && entry.compressed_size().is_none() {
                vec![target]
            } else {
                return Err(Error::PayloadNotFound {
                    entry: entry.path.to_string(),
                });
            }
        }
        FileType::SymbolicLink => {
            if entry.symlink_target().is_none() {
                return Ok(Vec::new());
            }
            #[cfg(unix)]
            {
                vec![target]
            }
            // Without symlink support a breadcrumb file is written
            // instead.
            #[cfg(not(unix))]
            {
                vec![link_breadcrumb(&target)]
            }
        }
        _ => Vec::new(),
    })
}

fn extract_one(
    reader: &mut ImageReader<'_>,
    entry: &Entry,
    out_dir: &Path,
    options: &ExtractOptions,
) -> Result<ExtractOutcome> {
    let Some(relative) = resolve_relative(entry, &options.path_mode) else {
        return Ok(ExtractOutcome::Skipped);
    };
    let target: PathBuf = out_dir.join(host_path(&relative)?);

    match entry.file_type {
        FileType::Directory => {
            std::fs::create_dir_all(&target).map_err(|source| Error::io(&target, source))?;
            apply_mode(&target, entry.mode)?;
            Ok(ExtractOutcome::Written(None))
        }
        FileType::Regular => {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
            }
            let recovery = if entry.payload.is_some() {
                let payload = reader.read(entry)?;
                let recovery =
                    (payload.location.resolution != PayloadResolution::Exact).then(|| {
                        ExtractRecovery {
                            path: entry.path.to_string(),
                            expected_record_offset: payload.location.expected_record_offset,
                            actual_record_offset: payload.location.actual_record_offset,
                            resolution: payload.location.resolution,
                        }
                    });
                match options.decode {
                    DecodeMode::Auto => {
                        write_file(
                            &target,
                            &payload.decode()?,
                            entry.mode,
                            options.existing_output,
                        )?;
                        if options.keep_stored && payload.stored_compressed {
                            write_file(
                                &stored_target(&target),
                                &payload.bytes,
                                entry.mode,
                                options.existing_output,
                            )?;
                        }
                    }
                    DecodeMode::Never => {
                        if payload.stored_compressed {
                            write_file(
                                &stored_target(&target),
                                &payload.bytes,
                                entry.mode,
                                options.existing_output,
                            )?;
                        } else {
                            write_file(
                                &target,
                                &payload.bytes,
                                entry.mode,
                                options.existing_output,
                            )?;
                        }
                    }
                }
                recovery
            } else if entry.size() == Some(0) && entry.compressed_size().is_none() {
                // A regular file explicitly recorded as empty.
                write_file(&target, &[], entry.mode, options.existing_output)?;
                None
            } else {
                return Err(Error::PayloadNotFound {
                    entry: entry.path.to_string(),
                });
            };
            Ok(ExtractOutcome::Written(recovery))
        }
        FileType::SymbolicLink => {
            let Some(link_target) = entry.symlink_target() else {
                return Ok(ExtractOutcome::Skipped);
            };
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
            }
            create_symlink_or_fallback(link_target, &target, options.existing_output)?;
            Ok(ExtractOutcome::Written(None))
        }
        // Devices and FIFOs need privileges and a live system; skip them.
        _ => Ok(ExtractOutcome::Skipped),
    }
}

fn write_file(
    target: &Path,
    bytes: &[u8],
    mode: u32,
    existing: ExistingOutputPolicy,
) -> Result<()> {
    match existing {
        ExistingOutputPolicy::Allow => {
            std::fs::write(target, bytes).map_err(|source| Error::io(target, source))?;
        }
        ExistingOutputPolicy::Refuse => {
            // Atomic no-clobber: a target that appeared after the
            // caller's preflight fails here instead of being truncated.
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .map_err(|source| Error::io(target, source))?;
            file.write_all(bytes)
                .map_err(|source| Error::io(target, source))?;
        }
    }
    apply_mode(target, mode)
}

/// The path a payload's stored (still compressed) bytes are written to:
/// the file name with `.Z` appended, e.g. `libfoo.so` becomes
/// `libfoo.so.Z`.
fn stored_target(target: &Path) -> PathBuf {
    match target.file_name().and_then(|name| name.to_str()) {
        Some(name) => target.with_file_name(format!("{name}.Z")),
        None => target.with_extension("Z"),
    }
}

/// Host filesystem naming rules for [`host_path`] validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostRules {
    Unix,
    Windows,
}

/// Converts a normalized IRIX path into a host-relative path, validating
/// every component against the host filesystem's naming rules.
///
/// [`IrixPath`] already guarantees the path cannot escape the output
/// directory; this additionally rejects components the host filesystem
/// cannot represent. On Unix that adds nothing beyond what [`IrixPath`]
/// already excludes (`/` and NUL); on Windows it rejects `\` and other
/// reserved characters, device names (`CON`, `COM1`, ...) and trailing
/// dots or spaces.
///
/// # Errors
///
/// Returns [`Error::UnsafePath`] if a component violates the host rules.
pub fn host_path(path: &IrixPath) -> Result<PathBuf> {
    let rules = if cfg!(windows) {
        HostRules::Windows
    } else {
        HostRules::Unix
    };
    host_path_with(path, rules)
}

fn host_path_with(path: &IrixPath, rules: HostRules) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.as_str().split('/') {
        if component.is_empty() {
            continue;
        }
        if !valid_component(component, rules) {
            return Err(Error::UnsafePath {
                path: format!("{path} (component {component:?} is invalid on this host)"),
            });
        }
        out.push(component);
    }
    Ok(out)
}

fn valid_component(component: &str, rules: HostRules) -> bool {
    match rules {
        HostRules::Unix => true,
        HostRules::Windows => {
            if component
                .chars()
                .any(|c| matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*') || c < ' ')
            {
                return false;
            }
            if component.ends_with('.') || component.ends_with(' ') {
                return false;
            }
            let stem = component.split('.').next().unwrap_or(component);
            let stem = stem.to_ascii_uppercase();
            !matches!(
                stem.as_str(),
                "CON"
                    | "PRN"
                    | "AUX"
                    | "NUL"
                    | "COM1"
                    | "COM2"
                    | "COM3"
                    | "COM4"
                    | "COM5"
                    | "COM6"
                    | "COM7"
                    | "COM8"
                    | "COM9"
                    | "LPT1"
                    | "LPT2"
                    | "LPT3"
                    | "LPT4"
                    | "LPT5"
                    | "LPT6"
                    | "LPT7"
                    | "LPT8"
                    | "LPT9"
            )
        }
    }
}

#[cfg(unix)]
fn apply_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions).map_err(|source| Error::io(path, source))
}

#[cfg(not(unix))]
fn apply_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn create_symlink_or_fallback(
    original: &str,
    link: &Path,
    existing: ExistingOutputPolicy,
) -> Result<()> {
    // A symbolic link never replaces an existing path, under either
    // policy: removing a regular file to make room for a link would
    // change the filesystem object type (not "overwrite" it), and
    // removing a pre-existing link — even a dangling one, which
    // `symlink_metadata` still detects — could retarget paths that
    // resolve through it. `symlink(2)` itself never overwrites; the
    // check below only produces a clearer error.
    if std::fs::symlink_metadata(link).is_ok() {
        return Err(Error::io(link, already_exists()));
    }
    match std::os::unix::fs::symlink(original, link) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            // Lost the race against a path that appeared after the
            // check: report the failure instead of writing a breadcrumb
            // next to it.
            Err(Error::io(link, source))
        }
        Err(_) => {
            // Filesystems without symlink support (e.g. some network or
            // FAT mounts): leave a plain-text breadcrumb instead of
            // pretending success.
            let meta = link_breadcrumb(link);
            write_breadcrumb(&meta, original, existing)
        }
    }
}

#[cfg(not(unix))]
fn create_symlink_or_fallback(
    original: &str,
    link: &Path,
    existing: ExistingOutputPolicy,
) -> Result<()> {
    let meta = link_breadcrumb(link);
    write_breadcrumb(&meta, original, existing)
}

/// Writes a symlink breadcrumb file, honoring the existing-output
/// policy: [`ExistingOutputPolicy::Refuse`] creates it atomically
/// without clobbering.
fn write_breadcrumb(meta: &Path, original: &str, existing: ExistingOutputPolicy) -> Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    match existing {
        ExistingOutputPolicy::Allow => {
            options.write(true).create(true).truncate(true);
        }
        ExistingOutputPolicy::Refuse => {
            options.write(true).create_new(true);
        }
    }
    let mut file = options
        .open(meta)
        .map_err(|source| Error::io(meta, source))?;
    writeln!(file, "{original}").map_err(|source| Error::io(meta, source))
}

/// The error a refused overwrite is reported with.
#[cfg(unix)]
fn already_exists() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "output path already exists",
    )
}

/// The breadcrumb path used when a symbolic link cannot be created:
/// the link path with `.link.txt` appended, e.g. `libfoo.so` becomes
/// `libfoo.so.link.txt`.
fn link_breadcrumb(link: &Path) -> PathBuf {
    match link.file_name().and_then(|name| name.to_str()) {
        Some(name) => link.with_file_name(format!("{name}.link.txt")),
        None => link.with_extension("link.txt"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_host_rules_accept_irix_names() {
        let path = IrixPath::new("usr/lib/libfoo.so").unwrap();
        let host = host_path_with(&path, HostRules::Unix).unwrap();
        assert_eq!(host, PathBuf::from("usr/lib/libfoo.so"));
        // Backslashes and colons are ordinary characters on Unix.
        let weird = IrixPath::new("a\\b:c").unwrap();
        assert!(host_path_with(&weird, HostRules::Unix).is_ok());
    }

    #[test]
    fn windows_host_rules_reject_host_syntax() {
        for raw in [
            "a\\b", "a:b", "a?b", "a*b", "foo.", "foo ", "CON", "com1", "lpt9.txt",
        ] {
            let path = IrixPath::new(raw).unwrap();
            assert!(
                host_path_with(&path, HostRules::Windows).is_err(),
                "{raw:?} should be rejected on Windows"
            );
        }
        let ok = IrixPath::new("usr/lib/libfoo.so").unwrap();
        assert!(host_path_with(&ok, HostRules::Windows).is_ok());
    }

    #[test]
    fn stored_target_appends_z_to_the_full_name() {
        assert_eq!(
            stored_target(Path::new("usr/lib/libfoo.so")),
            PathBuf::from("usr/lib/libfoo.so.Z")
        );
        assert_eq!(
            link_breadcrumb(Path::new("usr/lib/libfoo.so")),
            PathBuf::from("usr/lib/libfoo.so.link.txt")
        );
    }
}
