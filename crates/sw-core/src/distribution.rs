//! The top-level object: a whole SGI distribution directory.
//!
//! A distribution directory contains, per product, a descriptor (`eoe`),
//! an IDB (`eoe.idb`) and one archive per image (`eoe.sw`, `eoe.man`,
//! `eoe.sw32`, ...). It may also carry the miniroot support files `sa`
//! and `mr`.
//!
//! The logical tree (product → image → subsystem → entry) is built from
//! the IDB alone, because SGI names are self-describing
//! (`eoe.sw.unix` lives in image `eoe.sw`); the binary descriptor enriches
//! it with titles, versions, flags and rules once its grammar is decoded.
use crate::descriptor::model::{ProductDescriptor, Subsystem};
use crate::descriptor::parser as descriptor_parser;
use crate::diagnostic::Diagnostic;
use crate::error::{Error, Result};
use crate::idb::parser as idb_parser;
use crate::idb::{Entry, EntryId};
use crate::image::layout::{
    IMAGE_HEADER_SIZE, LayoutInput, compute_layout, validate_archive_header,
};
use crate::image::reader::ImageReader;
use crate::image::{Image, ImageArchive, ImageLayout};
use crate::mach::HardwareExpr;
use crate::mach::eval::HardwareProfile;
use crate::names::{ImageName, ProductName};
use crate::path::IrixPath;
use crate::query::{Query, QueryResult};
use crate::selection::{self, EntrySelection};
use std::path::{Path, PathBuf};

/// Miniroot support files that may sit next to the products.
#[derive(Debug, Clone, Default)]
pub struct DistributionSupportFiles {
    /// The standalone (`sa`) file, if present.
    pub sa: Option<PathBuf>,
    /// The miniroot (`mr`) file, if present.
    pub mr: Option<PathBuf>,
}

/// A whole distribution directory.
#[derive(Debug)]
pub struct Distribution {
    root: PathBuf,
    products: Vec<Product>,
    support_files: DistributionSupportFiles,
    diagnostics: Vec<Diagnostic>,
}

impl Distribution {
    /// Opens a distribution directory and builds the full logical model:
    /// every product's IDB is parsed, images and subsystems are assembled,
    /// and image layouts are computed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotADistribution`] if `path` is not a directory, or
    /// [`Error::Io`] if the directory cannot be listed. Individual product
    /// problems are reported as diagnostics instead.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let root = path.as_ref().to_path_buf();
        if !root.is_dir() {
            return Err(Error::NotADistribution { path: root });
        }

        let mut listing = std::fs::read_dir(&root).map_err(|source| Error::io(&root, source))?;
        let mut idb_files: Vec<PathBuf> = Vec::new();
        while let Some(entry) = listing
            .next()
            .transpose()
            .map_err(|e| Error::io(&root, e))?
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "idb") {
                idb_files.push(path);
            }
        }
        idb_files.sort();

        let mut diagnostics = Vec::new();
        let mut products = Vec::new();
        for idb_file in idb_files {
            match build_product(&root, &idb_file) {
                Ok(product) => products.push(product),
                Err(error) => diagnostics.push(Diagnostic::error(
                    format!("skipping product: {error}"),
                    Some(idb_file.display().to_string()),
                )),
            }
        }

        let support_files = DistributionSupportFiles {
            sa: existing_file(root.join("sa")),
            mr: existing_file(root.join("mr")),
        };

        Ok(Distribution {
            root,
            products,
            support_files,
            diagnostics,
        })
    }

    /// The distribution directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// All products, sorted by file name.
    pub fn products(&self) -> &[Product] {
        &self.products
    }

    /// Looks up a product by name, e.g. `eoe1`.
    pub fn product(&self, name: &str) -> Option<&Product> {
        self.products.iter().find(|p| p.name.as_str() == name)
    }

    /// Miniroot support files.
    pub fn support_files(&self) -> &DistributionSupportFiles {
        &self.support_files
    }

    /// Non-fatal problems found while opening the distribution.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Runs a search query across all products.
    pub fn find(&self, query: &Query) -> QueryResult<'_> {
        let mut result = QueryResult::default();
        for product in &self.products {
            result
                .entries
                .extend(product.entries.iter().filter(|e| query.matches(e)));
        }
        result
    }

    /// Selects the entries applicable to a hardware target, across all
    /// products.
    pub fn select(&self, profile: &HardwareProfile) -> EntrySelection<'_> {
        let mut selection = EntrySelection::default();
        for product in &self.products {
            let mut product_selection = selection::select_product(product, profile);
            selection.selected.append(&mut product_selection.selected);
            selection.conflicts.append(&mut product_selection.conflicts);
        }
        selection
    }

    /// Creates a payload reader bound to this distribution.
    pub fn image_reader(&self) -> ImageReader<'_> {
        ImageReader::new(self)
    }
}

/// One product of a distribution.
#[derive(Debug)]
pub struct Product {
    /// Product name, e.g. `eoe`.
    pub name: ProductName,
    /// Path of the descriptor file (e.g. `dist/eoe`); may not exist.
    pub descriptor_file: PathBuf,
    /// The parsed descriptor, if the file exists and its header is valid.
    ///
    /// Only the header is decoded so far; once the binary body grammar is
    /// reverse engineered, this is where titles, versions, flags and rules
    /// will surface.
    pub descriptor: Option<ProductDescriptor>,
    /// Path of the IDB file (e.g. `dist/eoe.idb`).
    pub idb_file: PathBuf,
    /// Product title from the descriptor.
    pub title: Option<String>,
    /// Hardware applicability expressions (OR-ed).
    pub mach: Vec<HardwareExpr>,
    /// Cut points recorded in the descriptor.
    pub cutpoints: Vec<IrixPath>,
    /// Images, in first-appearance order in the IDB.
    pub images: Vec<Image>,
    /// All entries, in IDB order.
    pub entries: Vec<Entry>,
    /// Non-fatal problems found while building this product.
    pub diagnostics: Vec<Diagnostic>,
}

impl Product {
    /// Looks up an entry by id.
    pub fn entry(&self, id: EntryId) -> Option<&Entry> {
        self.entries.get(id.0)
    }
}

fn existing_file(path: PathBuf) -> Option<PathBuf> {
    path.is_file().then_some(path)
}

/// Builds one product from its IDB (and, where possible, its descriptor).
fn build_product(root: &Path, idb_file: &Path) -> Result<Product> {
    let stem = idb_file
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::NotADistribution {
            path: idb_file.to_path_buf(),
        })?;
    let name = ProductName::new(stem)?;
    let descriptor_file = root.join(stem);

    let mut diagnostics = Vec::new();

    let descriptor = if descriptor_file.is_file() {
        let bytes = std::fs::read(&descriptor_file).map_err(|e| Error::io(&descriptor_file, e))?;
        match descriptor_parser::parse(&bytes, &descriptor_file, &mut diagnostics) {
            Ok(descriptor) => Some(descriptor),
            Err(error) => {
                diagnostics.push(Diagnostic::error(
                    error.to_string(),
                    Some(descriptor_file.display().to_string()),
                ));
                None
            }
        }
    } else {
        diagnostics.push(Diagnostic::warning(
            "product has no descriptor file",
            Some(idb_file.display().to_string()),
        ));
        None
    };

    let idb_bytes = std::fs::read(idb_file).map_err(|e| Error::io(idb_file, e))?;
    let mut entries = idb_parser::parse(&idb_bytes, idb_file, &mut diagnostics)?;

    let mut images = build_tree(root, &entries, &mut diagnostics);
    compute_layouts(&mut images, &mut entries);

    Ok(Product {
        name,
        descriptor_file,
        descriptor,
        idb_file: idb_file.to_path_buf(),
        title: None,
        mach: Vec::new(),
        cutpoints: Vec::new(),
        images,
        entries,
        diagnostics,
    })
}

/// Assembles the image/subsystem tree from entry subsystem names.
fn build_tree(root: &Path, entries: &[Entry], diagnostics: &mut Vec<Diagnostic>) -> Vec<Image> {
    let mut images: Vec<Image> = Vec::new();

    for entry in entries {
        let image_name: ImageName = entry.subsystem.image_name();
        let image_index = match images.iter().position(|i| i.name == image_name) {
            Some(index) => index,
            None => {
                let path = root.join(image_name.file_name());
                let file_size = match std::fs::metadata(&path) {
                    Ok(metadata) => match validate_archive_header(&path) {
                        Ok(()) => metadata.len(),
                        Err(error) => {
                            diagnostics.push(Diagnostic::error(
                                format!("ignoring image archive: {error}"),
                                Some(path.display().to_string()),
                            ));
                            0
                        }
                    },
                    Err(_) => {
                        diagnostics.push(Diagnostic::warning(
                            format!("image archive {} is missing", image_name.file_name()),
                            Some(path.display().to_string()),
                        ));
                        0
                    }
                };
                images.push(Image {
                    name: image_name,
                    title: None,
                    version: None,
                    order: None,
                    mach: Vec::new(),
                    archive: ImageArchive {
                        path,
                        file_size,
                        layout: ImageLayout {
                            header_size: IMAGE_HEADER_SIZE,
                            payloads: Vec::new(),
                        },
                    },
                    subsystems: Vec::new(),
                });
                images.len() - 1
            }
        };

        let image = &mut images[image_index];
        let subsystem_index = match image
            .subsystems
            .iter()
            .position(|s| s.name == entry.subsystem)
        {
            Some(index) => index,
            None => {
                image.subsystems.push(Subsystem {
                    name: entry.subsystem.clone(),
                    title: None,
                    mach: Vec::new(),
                    flags: Default::default(),
                    autominiroot: Vec::new(),
                    rules: Default::default(),
                    entry_ids: Vec::new(),
                });
                image.subsystems.len() - 1
            }
        };
        image.subsystems[subsystem_index].entry_ids.push(entry.id);
    }

    images
}

/// Computes the layout of every image and attaches payload locators to
/// the payload-bearing entries.
fn compute_layouts(images: &mut [Image], entries: &mut [Entry]) {
    for image in images {
        let bearing: Vec<EntryId> = entries
            .iter()
            .filter(|e| e.subsystem.image_name() == image.name && e.compressed_size().is_some())
            .map(|e| e.id)
            .collect();

        let inputs: Vec<LayoutInput> = bearing
            .iter()
            .map(|id| {
                let entry = &entries[id.0];
                LayoutInput {
                    raw_name: entry.raw_path.clone(),
                    encoded_size: entry.encoded_size(),
                }
            })
            .collect();

        let locators = compute_layout(&image.name, &inputs);
        for (id, locator) in bearing.iter().zip(locators.iter().cloned()) {
            entries[id.0].payload = Some(locator);
        }
        image.archive.layout.payloads = locators;
    }
}
