//! The CXX bridge itself: an opaque [`Backend`] handle plus the plain
//! data structs returned across the boundary.
//!
//! All `sw-core` errors are propagated to C++ as CXX exceptions; the
//! bridge contains no business logic of its own.
use sw_core::distribution::Distribution;

#[cxx::bridge(namespace = "sw")]
mod ffi {
    /// Counts describing a freshly opened distribution.
    struct DistributionSummary {
        /// Number of products found in the distribution.
        product_count: u64,
        /// Number of non-fatal problems reported while opening it.
        diagnostic_count: u64,
    }

    extern "Rust" {
        /// Opaque handle to the Rust backend state.
        type Backend;

        /// Creates an empty backend with no distribution loaded.
        fn new_backend() -> Box<Backend>;

        /// Opens the distribution directory at `path`.
        ///
        /// The previously loaded distribution is replaced only after
        /// the new one has been opened successfully; a failed open
        /// leaves the old state untouched.
        fn open_distribution(self: &mut Backend, path: &str) -> Result<DistributionSummary>;
    }
}

/// Rust-side backend state behind the opaque CXX handle.
pub struct Backend {
    distribution: Option<Distribution>,
}

fn new_backend() -> Box<Backend> {
    Box::new(Backend { distribution: None })
}

impl Backend {
    fn open_distribution(
        &mut self,
        path: &str,
    ) -> sw_core::error::Result<ffi::DistributionSummary> {
        // Open first, replace second: a failed open must not destroy
        // the distribution that is already loaded.
        let distribution = Distribution::open(path)?;
        let summary = ffi::DistributionSummary {
            product_count: distribution.products().len() as u64,
            diagnostic_count: distribution.diagnostics().len() as u64,
        };
        self.distribution = Some(distribution);
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn temp_root(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "sw-gui-bridge-test-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// Writes a minimal synthetic distribution: one valid product
    /// (`test`, IDB only) plus one product whose file name is not a
    /// valid product name, which surfaces as a distribution-level
    /// diagnostic.
    fn write_synthetic_dist(root: &Path) {
        std::fs::write(
            root.join("test.idb"),
            "f 0644 root sys hello.txt src/hello.txt test.sw.unix sum(1) size(5) cmpsize(0)\n",
        )
        .unwrap();
        std::fs::write(root.join("not.a.product.idb"), "garbage\n").unwrap();
    }

    #[test]
    fn new_backend_has_no_distribution() {
        let backend = new_backend();
        assert!(backend.distribution.is_none());
    }

    #[test]
    fn open_missing_directory_fails() {
        let mut backend = new_backend();
        let missing = std::env::temp_dir().join("sw-gui-bridge-test-definitely-missing");
        assert!(!missing.exists());
        let result = backend.open_distribution(missing.to_str().unwrap());
        assert!(result.is_err());
        assert!(backend.distribution.is_none());
    }

    #[test]
    fn open_synthetic_distribution_reports_counts() {
        let root = temp_root("synthetic");
        write_synthetic_dist(&root);

        let mut backend = new_backend();
        let summary = backend.open_distribution(root.to_str().unwrap()).unwrap();
        assert_eq!(summary.product_count, 1);
        assert_eq!(summary.diagnostic_count, 1);
        assert!(backend.distribution.is_some());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_open_keeps_previous_distribution() {
        let root = temp_root("keep");
        write_synthetic_dist(&root);

        let mut backend = new_backend();
        backend.open_distribution(root.to_str().unwrap()).unwrap();

        let missing = root.join("missing");
        assert!(
            backend
                .open_distribution(missing.to_str().unwrap())
                .is_err()
        );
        assert!(backend.distribution.is_some());

        let _ = std::fs::remove_dir_all(&root);
    }
}
