//! The top-level object: a whole SGI distribution directory.
//!
//! A distribution directory contains, per product, a descriptor (`eoe`),
//! an IDB (`eoe.idb`) and one archive per image (`eoe.sw`, `eoe.man`,
//! `eoe.sw32`, ...). It may also carry the miniroot support files `sa`
//! and `mr`.
//!
//! The logical tree (product → image → subsystem) is built from the
//! binary descriptor, which is the hierarchy authority; IDB entries are
//! then attached to the subsystems they name. A product whose descriptor
//! is missing or unreadable falls back to a tree derived from the IDB
//! alone, with every subsystem marked accordingly.
use crate::descriptor::model::{
    Cutpoint, HardwareRestrictions, ProductDescriptor, Subsystem, SubsystemPresence,
};
use crate::descriptor::parser as descriptor_parser;
use crate::descriptor::semantics;
use crate::diagnostic::Diagnostic;
use crate::error::{Error, Result};
use crate::idb::parser as idb_parser;
use crate::idb::{Entry, EntryId};
use crate::image::layout::{
    IMAGE_HEADER_SIZE, LayoutInput, compute_layout, validate_archive_header,
};
use crate::image::reader::ImageReader;
use crate::image::{Image, ImageArchive, ImageLayout};
use crate::mach::candidates::HardwareCandidateSet;
use crate::mach::eval::HardwareProfile;
use crate::names::{ImageName, ProductName};
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

/// The canonical identity of an [`Entry`] within one [`Distribution`]
/// instance.
///
/// An [`EntryId`] alone identifies an entry only within its owning
/// product; several products may hold an entry with the same id, and a
/// record's subsystem name says nothing about ownership (an IDB record
/// may name a foreign subsystem). `EntryKey` pairs the entry id with
/// the position of the owning product in [`Distribution::products`],
/// making it unique across the whole distribution.
///
/// Stability contract: an `EntryKey` is valid for the lifetime of the
/// [`Distribution`] instance it was obtained from. It is a session-local
/// identity, not a permanent database id: it is *not* stable across
/// reopening the same directory, and never comparable across different
/// `Distribution` instances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryKey {
    /// Position of the owning product in [`Distribution::products`].
    pub product_index: usize,
    /// The entry's id within its owning product (IDB order, 0-based).
    pub entry_id: EntryId,
}

/// An entry together with its canonical distribution-wide identity.
///
/// This is the shape every distribution-wide listing (search results,
/// hardware selection, extraction plans) carries: the borrowed entry
/// plus the [`EntryKey`] of the product that actually owns it, so no
/// caller ever has to re-derive ownership from a subsystem name or a
/// pointer.
///
/// Only `sw-core` can pair a key with an entry — the
/// struct cannot be constructed outside this crate — so `key` always
/// names the product that actually owns `entry`.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct LocatedEntry<'a> {
    /// The entry's canonical identity within its distribution.
    pub key: EntryKey,
    /// The entry itself, borrowed from its owning product.
    pub entry: &'a Entry,
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
    /// Products are discovered as the *union* of IDB files (`<name>.idb`)
    /// and descriptor files (extensionless `<name>` with a `pd001V`
    /// header): a product with only one of the two is kept, mirroring
    /// the descriptor/IDB asymmetry real media allow.
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
        let mut idb_files: std::collections::BTreeMap<String, PathBuf> =
            std::collections::BTreeMap::new();
        let mut stems: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        while let Some(entry) = listing
            .next()
            .transpose()
            .map_err(|e| Error::io(&root, e))?
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "idb") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    stems.insert(stem.to_string());
                    idb_files.insert(stem.to_string(), path);
                }
            } else if path.extension().is_none()
                && path.is_file()
                && has_descriptor_header(&path)
                && let Some(stem) = path.file_name().and_then(|s| s.to_str())
            {
                stems.insert(stem.to_string());
            }
        }

        let mut diagnostics = Vec::new();
        let mut products = Vec::new();
        for stem in stems {
            match build_product(&root, &stem, idb_files.get(&stem)) {
                Ok(product) => products.push(product),
                Err(error) => diagnostics.push(Diagnostic::error(
                    format!("skipping product: {error}"),
                    Some(stem),
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

    /// Resolves a canonical entry key to the entry it identifies.
    ///
    /// Both halves of the key are bounds-checked. Keys originating from
    /// another `Distribution` instance are outside the API contract (see
    /// the stability contract on [`EntryKey`]): when their numeric
    /// halves happen to be valid in this distribution, they identify a
    /// different entry of *this* distribution, not an error.
    pub fn entry(&self, key: EntryKey) -> Option<LocatedEntry<'_>> {
        let entry = self.products.get(key.product_index)?.entry(key.entry_id)?;
        Some(LocatedEntry { key, entry })
    }

    /// Miniroot support files.
    pub fn support_files(&self) -> &DistributionSupportFiles {
        &self.support_files
    }

    /// Non-fatal problems found while opening the distribution.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Every diagnostic of the distribution: the distribution-level ones
    /// first, then each product's in product order (each in stored
    /// order).
    ///
    /// This is the aggregation every frontend should use; whether a
    /// diagnostic was recorded at the distribution or the product level
    /// is a storage detail, not semantics.
    pub fn all_diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .chain(self.products.iter().flat_map(|p| p.diagnostics.iter()))
    }

    /// The total number of diagnostics, distribution- and product-level.
    pub fn diagnostic_count(&self) -> usize {
        self.all_diagnostics().count()
    }

    /// The hardware attribute values the distribution's MACH expressions
    /// compare against, grouped by attribute, in first-appearance order.
    ///
    /// Candidates come exclusively from successfully parsed expressions
    /// (product, image, subsystem and entry level); payloads that could
    /// not be parsed contribute nothing.
    pub fn hardware_candidates(&self) -> Vec<HardwareCandidateSet> {
        crate::mach::candidates::hardware_candidates(self)
    }

    /// Runs a search query across all products.
    ///
    /// Hits keep their owning-product identity: the result is in
    /// distribution product order, each product in IDB order, and
    /// duplicate paths are never deduplicated.
    pub fn find(&self, query: &Query) -> QueryResult<'_> {
        let mut result = QueryResult::default();
        for (product_index, product) in self.products.iter().enumerate() {
            result
                .entries
                .extend(
                    product
                        .entries
                        .iter()
                        .filter(|e| query.matches(e))
                        .map(|entry| LocatedEntry {
                            key: EntryKey {
                                product_index,
                                entry_id: entry.id,
                            },
                            entry,
                        }),
                );
        }
        result
    }

    /// Selects the entries applicable to a hardware target, across all
    /// products.
    ///
    /// Selected entries and conflict candidates keep their
    /// owning-product identity.
    pub fn select(&self, profile: &HardwareProfile) -> EntrySelection<'_> {
        let mut selection = EntrySelection::default();
        for (product_index, product) in self.products.iter().enumerate() {
            let mut product_selection = selection::select_product(product_index, product, profile);
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
    /// The parsed descriptor, if the file exists and parses exactly.
    pub descriptor: Option<ProductDescriptor>,
    /// Path of the IDB file (e.g. `dist/eoe.idb`), if the product has
    /// one.
    pub idb_file: Option<PathBuf>,
    /// Product title from the descriptor.
    pub title: Option<String>,
    /// Hardware applicability from the descriptor's `mach` attribute
    /// blobs; `None` when there is no parsed descriptor.
    pub mach: Option<HardwareRestrictions>,
    /// Cut points recorded in the descriptor.
    pub cutpoints: Vec<Cutpoint>,
    /// Images, in descriptor order (or first-appearance order in the IDB
    /// when the product has no parsed descriptor).
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

/// Whether a file starts with the `pd001V` descriptor magic.
fn has_descriptor_header(path: &Path) -> bool {
    let mut prefix = [0u8; 6];
    use std::io::Read as _;
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut prefix))
        .is_ok_and(|()| &prefix == b"pd001V")
}

/// Builds one product from its IDB (and, where possible, its descriptor).
fn build_product(root: &Path, stem: &str, idb_file: Option<&PathBuf>) -> Result<Product> {
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
            idb_file.map(|path| path.display().to_string()),
        ));
        None
    };

    let mut entries = match idb_file {
        Some(idb_file) => {
            let idb_bytes = std::fs::read(idb_file).map_err(|e| Error::io(idb_file, e))?;
            idb_parser::parse(&idb_bytes, idb_file, &mut diagnostics)?
        }
        None => {
            diagnostics.push(Diagnostic::warning(
                "product has a descriptor but no IDB file",
                Some(descriptor_file.display().to_string()),
            ));
            Vec::new()
        }
    };

    if let Some(descriptor) = &descriptor
        && descriptor.name != name.as_str()
    {
        diagnostics.push(Diagnostic::warning(
            format!(
                "descriptor product name {:?} does not match file name {:?}",
                descriptor.name,
                name.as_str()
            ),
            Some(descriptor_file.display().to_string()),
        ));
    }

    let mut images = match &descriptor {
        Some(descriptor) => tree_from_descriptor(root, &name, descriptor, &mut diagnostics),
        None => Vec::new(),
    };
    attach_entries(
        root,
        &mut images,
        &entries,
        descriptor.is_some(),
        &mut diagnostics,
    );
    compute_layouts(&mut images, &mut entries);

    let (title, mach, cutpoints) = match &descriptor {
        Some(descriptor) => (
            (!descriptor.text.effective().is_empty())
                .then(|| descriptor.text.effective().to_string()),
            Some(semantics::mach_expressions(
                &descriptor.attributes,
                &descriptor_file.display().to_string(),
                &mut diagnostics,
            )),
            semantics::cutpoints(
                &descriptor.attributes,
                &descriptor_file.display().to_string(),
                &mut diagnostics,
            ),
        ),
        None => (None, None, Vec::new()),
    };

    Ok(Product {
        name,
        descriptor_file,
        descriptor,
        idb_file: idb_file.cloned(),
        title,
        mach,
        cutpoints,
        images,
        entries,
        diagnostics,
    })
}

/// Builds the image/subsystem tree from the descriptor, the hierarchy
/// authority. Entries are attached later by [`attach_entries`].
fn tree_from_descriptor(
    root: &Path,
    product: &ProductName,
    descriptor: &ProductDescriptor,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Image> {
    let mut images = Vec::new();
    for image_record in &descriptor.images {
        let image_name = match ImageName::from_parts(product.clone(), image_record.name.clone()) {
            Ok(name) => name,
            Err(error) => {
                diagnostics.push(Diagnostic::error(
                    format!("skipping descriptor image record: {error}"),
                    None,
                ));
                continue;
            }
        };
        let mut subsystems = Vec::new();
        for subsystem_record in &image_record.subsystems {
            let qualified = format!("{product}.{}.{}", image_record.name, subsystem_record.name);
            let name = match crate::names::SubsystemName::parse(&qualified) {
                Ok(name) => name,
                Err(error) => {
                    diagnostics.push(Diagnostic::error(
                        format!("skipping descriptor subsystem record: {error}"),
                        None,
                    ));
                    continue;
                }
            };
            let qualified_name = qualified.clone();
            subsystems.push(Subsystem {
                name,
                title: (!subsystem_record.text.effective().is_empty())
                    .then(|| subsystem_record.text.effective().to_string()),
                mapping: (!subsystem_record.mapping.is_empty())
                    .then(|| subsystem_record.mapping.clone()),
                presence: SubsystemPresence {
                    descriptor: true,
                    idb: false,
                },
                mach: Some(semantics::mach_expressions(
                    &subsystem_record.attributes,
                    &qualified_name,
                    &mut *diagnostics,
                )),
                flags: Some(semantics::decode_flags(
                    subsystem_record.flags_raw,
                    &subsystem_record.attributes,
                    &qualified_name,
                    &mut *diagnostics,
                )),
                autominiroot: Some(semantics::autominiroot(
                    &subsystem_record.attributes,
                    &qualified_name,
                    &mut *diagnostics,
                )),
                rules: Some(semantics::decode_rules(
                    subsystem_record,
                    &qualified_name,
                    &mut *diagnostics,
                )),
                entry_ids: Vec::new(),
            });
        }
        images.push(Image {
            title: (!image_record.text.effective().is_empty())
                .then(|| image_record.text.effective().to_string()),
            version: Some(image_record.version),
            order: Some(i32::from(image_record.order)),
            mach: Some(semantics::mach_expressions(
                &image_record.attributes,
                &image_name.file_name(),
                &mut *diagnostics,
            )),
            archive: open_image_archive(root, &image_name, diagnostics),
            name: image_name,
            subsystems,
        });
    }
    images
}

/// Attaches every IDB entry to the subsystem it names.
///
/// An entry naming a subsystem the descriptor does not declare creates a
/// synthetic IDB-only subsystem (with a warning); such entries must not
/// be dropped. When there is no descriptor at all, the whole tree is
/// derived from the IDB instead, as the only available authority.
fn attach_entries(
    root: &Path,
    images: &mut Vec<Image>,
    entries: &[Entry],
    has_descriptor: bool,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for entry in entries {
        let image_name: ImageName = entry.subsystem.image_name();
        let image_index = match images.iter().position(|i| i.name == image_name) {
            Some(index) => index,
            None => {
                if has_descriptor {
                    diagnostics.push(Diagnostic::warning(
                        format!(
                            "IDB entry {} names image {} which the descriptor does not declare",
                            entry.path, entry.subsystem
                        ),
                        Some(format!(
                            "{}:{}",
                            entry.origin.idb_path.display(),
                            entry.origin.line_number
                        )),
                    ));
                }
                images.push(Image {
                    archive: open_image_archive(root, &image_name, diagnostics),
                    name: image_name,
                    title: None,
                    version: None,
                    order: None,
                    mach: None,
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
                if has_descriptor {
                    diagnostics.push(Diagnostic::warning(
                        format!(
                            "IDB entry {} names subsystem {} which the descriptor \
                             does not declare",
                            entry.path, entry.subsystem
                        ),
                        Some(format!(
                            "{}:{}",
                            entry.origin.idb_path.display(),
                            entry.origin.line_number
                        )),
                    ));
                }
                image.subsystems.push(Subsystem {
                    name: entry.subsystem.clone(),
                    title: None,
                    mapping: None,
                    presence: SubsystemPresence {
                        descriptor: false,
                        idb: true,
                    },
                    // IDB-only subsystem: there is no descriptor record,
                    // so nothing descriptor-derived is known.
                    mach: None,
                    flags: None,
                    autominiroot: None,
                    rules: None,
                    entry_ids: Vec::new(),
                });
                image.subsystems.len() - 1
            }
        };
        let subsystem = &mut image.subsystems[subsystem_index];
        subsystem.presence.idb = true;
        subsystem.entry_ids.push(entry.id);
    }
}

/// Opens the archive file of one image, reporting problems as
/// diagnostics and yielding an empty layout.
fn open_image_archive(
    root: &Path,
    image_name: &ImageName,
    diagnostics: &mut Vec<Diagnostic>,
) -> ImageArchive {
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
    ImageArchive {
        path,
        file_size,
        layout: ImageLayout {
            header_size: IMAGE_HEADER_SIZE,
            payloads: Vec::new(),
        },
    }
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
