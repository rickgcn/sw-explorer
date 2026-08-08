//! Normalized IRIX destination paths.

use crate::error::{Error, Result};
use std::path::PathBuf;

/// A normalized, extraction-safe IRIX path.
///
/// IDB records write destination paths in several spellings:
///
/// ```text
/// /usr/lib/foo
/// usr/lib/foo
/// ./usr/lib/foo
/// usr/gfx/ucode/MG/../ge11.bin
/// ```
///
/// `IrixPath` normalizes all of these to the canonical relative form by
/// resolving `.` and `..` lexically, exactly like a pathname walk would
/// (`usr/gfx/ucode/MG/../ge11.bin` becomes `usr/gfx/ucode/ge11.bin`).
/// Real distributions rely on this. Anything that would escape the
/// distribution root is rejected. The empty path represents the
/// distribution root itself (IDB uses `.` for it).
///
/// The raw spelling is kept separately on
/// [`crate::idb::Entry::raw_path`], because image archive record matching
/// needs the original bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IrixPath(String);

impl IrixPath {
    /// Normalizes a raw IDB destination path, resolving `.` and `..`
    /// lexically.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsafePath`] if the path escapes the distribution
    /// root (`..` above the root) or contains a NUL byte.
    pub fn new(raw: &str) -> Result<Self> {
        if raw.contains('\0') {
            return Err(Error::UnsafePath {
                path: raw.to_string(),
            });
        }
        let mut components: Vec<&str> = Vec::new();
        for component in raw.split('/') {
            match component {
                "" | "." => {}
                ".." => match components.pop() {
                    Some(_) => {}
                    None => {
                        return Err(Error::UnsafePath {
                            path: raw.to_string(),
                        });
                    }
                },
                other => components.push(other),
            }
        }
        Ok(IrixPath(components.join("/")))
    }

    /// The normalized path, relative to the distribution root.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this path denotes the distribution root (IDB `.`).
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The final component, if any.
    pub fn file_name(&self) -> Option<&str> {
        self.0.rsplit('/').next().filter(|s| !s.is_empty())
    }

    /// The parent path, or `None` if this path is the root or has a single
    /// component.
    pub fn parent(&self) -> Option<IrixPath> {
        let (parent, _) = self.0.rsplit_once('/')?;
        Some(IrixPath(parent.to_string()))
    }

    /// Strips a leading path prefix, component-wise.
    ///
    /// Stripping the path itself yields the (empty) root path, which maps
    /// to the output directory during extraction. Returns `None` only
    /// when this path is not below `prefix`.
    pub fn strip_prefix(&self, prefix: &IrixPath) -> Option<IrixPath> {
        if prefix.is_root() {
            return (!self.is_root()).then(|| self.clone());
        }
        if self == prefix {
            return Some(IrixPath(String::new()));
        }
        let rest = self.0.strip_prefix(prefix.as_str())?;
        let rest = rest.strip_prefix('/')?;
        Some(IrixPath(rest.to_string()))
    }

    /// Converts to a relative host path for extraction.
    pub fn to_path_buf(&self) -> PathBuf {
        let mut out = PathBuf::new();
        for component in self.0.split('/') {
            if !component.is_empty() {
                out.push(component);
            }
        }
        out
    }
}

impl std::fmt::Display for IrixPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_spellings() {
        assert_eq!(
            IrixPath::new("/usr/lib/foo").unwrap().as_str(),
            "usr/lib/foo"
        );
        assert_eq!(
            IrixPath::new("usr/lib/foo").unwrap().as_str(),
            "usr/lib/foo"
        );
        assert_eq!(
            IrixPath::new("./usr/lib/foo").unwrap().as_str(),
            "usr/lib/foo"
        );
        assert_eq!(
            IrixPath::new("usr//lib/./foo/").unwrap().as_str(),
            "usr/lib/foo"
        );
    }

    #[test]
    fn dot_is_root() {
        let root = IrixPath::new(".").unwrap();
        assert!(root.is_root());
        assert_eq!(root.as_str(), "");
    }

    #[test]
    fn resolves_parent_components_lexically() {
        assert_eq!(
            IrixPath::new("usr/gfx/ucode/MG/../ge11.bin")
                .unwrap()
                .as_str(),
            "usr/gfx/ucode/ge11.bin"
        );
        assert_eq!(IrixPath::new("a/b/../../c").unwrap().as_str(), "c");
    }

    #[test]
    fn rejects_escapes_and_nul() {
        assert!(IrixPath::new("../etc/passwd").is_err());
        assert!(IrixPath::new("usr/../../etc").is_err());
        assert!(IrixPath::new("usr/lib\0foo").is_err());
    }

    #[test]
    fn file_name_and_parent() {
        let p = IrixPath::new("usr/lib/foo").unwrap();
        assert_eq!(p.file_name(), Some("foo"));
        assert_eq!(p.parent().unwrap().as_str(), "usr/lib");
        assert!(IrixPath::new("foo").unwrap().parent().is_none());
    }

    #[test]
    fn hidden_files_are_normal_components() {
        let p = IrixPath::new(".cshrc").unwrap();
        assert_eq!(p.as_str(), ".cshrc");
        assert_eq!(p.file_name(), Some(".cshrc"));
    }

    #[test]
    fn strips_component_prefixes() {
        let path = IrixPath::new("usr/lib/foo").unwrap();
        let prefix = IrixPath::new("usr").unwrap();
        assert_eq!(path.strip_prefix(&prefix).unwrap().as_str(), "lib/foo");
        let prefix = IrixPath::new("usr/lib").unwrap();
        assert_eq!(path.strip_prefix(&prefix).unwrap().as_str(), "foo");

        // A shared string prefix is not a component prefix.
        let wrong = IrixPath::new("usr2/x").unwrap();
        assert!(wrong.strip_prefix(&IrixPath::new("usr").unwrap()).is_none());
        // Stripping the prefix itself yields the output root.
        assert!(prefix.strip_prefix(&prefix).unwrap().is_root());
        // The root prefix strips to the path itself.
        let root = IrixPath::new(".").unwrap();
        assert_eq!(path.strip_prefix(&root).unwrap().as_str(), "usr/lib/foo");
    }
}
