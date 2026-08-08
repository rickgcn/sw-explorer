//! The product installation database (IDB).
//!
//! Each line of a `product.idb` file describes one filesystem object:
//!
//! ```text
//! f 0755 root sys .cshrc work/irix/cmd/adm/root.cshrc eoe1.sw.unix sum(61435) size(601) cmpsize(465)
//! ```
//!
//! The fields are: entry type, octal mode, owner, group, destination path,
//! source path, subsystem, and a free-form attribute list parsed by
//! [`attribute`].
pub mod attribute;
pub(crate) mod parser;

use crate::idb::attribute::{ConfigMode, IdbAttribute};
use crate::image::PayloadLocator;
use crate::mach::HardwareExpr;
use crate::names::SubsystemName;
use crate::path::IrixPath;
use std::path::PathBuf;

/// Stable identifier of an [`Entry`] within its product.
///
/// Subsystems reference entries through `EntryId` instead of copying them;
/// all entries live in `Product::entries`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryId(pub usize);

/// The kind of filesystem object an IDB record describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// `f` — a regular file.
    Regular,
    /// `d` — a directory.
    Directory,
    /// `l` — a symbolic link.
    SymbolicLink,
    /// `b` — a block special device.
    BlockDevice,
    /// `c` — a character special device.
    CharacterDevice,
    /// `p` — a named pipe (FIFO).
    Fifo,
    /// Any other type letter, preserved for forward compatibility.
    Unknown(char),
}

impl FileType {
    /// Maps an IDB type letter to a file type.
    pub fn from_letter(letter: char) -> Self {
        match letter {
            'f' => FileType::Regular,
            'd' => FileType::Directory,
            'l' => FileType::SymbolicLink,
            'b' => FileType::BlockDevice,
            'c' => FileType::CharacterDevice,
            'p' => FileType::Fifo,
            other => FileType::Unknown(other),
        }
    }
}

/// Where an entry came from, for diagnostics and format research.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryOrigin {
    /// The IDB file the entry was parsed from.
    pub idb_path: PathBuf,
    /// 1-based line number within that file.
    pub line_number: usize,
    /// The original line, Latin-1 decoded.
    pub raw_line: String,
}

/// One IDB record: a file, directory, link or similar object.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Stable identifier within the owning product.
    pub id: EntryId,
    /// Kind of filesystem object.
    pub file_type: FileType,
    /// Permission bits, parsed from the octal IDB field.
    pub mode: u32,
    /// Owner name, e.g. `root`.
    pub owner: String,
    /// Group name, e.g. `sys`.
    pub group: String,
    /// Normalized destination path, used for browsing and safe extraction.
    pub path: IrixPath,
    /// Destination path exactly as written in the IDB, used for image
    /// record matching.
    pub raw_path: String,
    /// Build-tree source path recorded by `gendist`.
    pub source_path: String,
    /// The subsystem this entry belongs to.
    pub subsystem: SubsystemName,
    /// Ordered attribute list; the lossless source of truth.
    pub attributes: Vec<IdbAttribute>,
    /// Where the entry's bytes live in its image archive, if it carries
    /// payload data.
    pub payload: Option<PayloadLocator>,
    /// Parse origin of this entry.
    pub origin: EntryOrigin,
}

impl Entry {
    /// `size(...)` attribute, if present.
    pub fn size(&self) -> Option<u64> {
        self.attributes.iter().find_map(|a| match a {
            IdbAttribute::Size(size) => Some(*size),
            _ => None,
        })
    }

    /// `cmpsize(...)` attribute, if present.
    pub fn compressed_size(&self) -> Option<u64> {
        self.attributes.iter().find_map(|a| match a {
            IdbAttribute::CompressedSize(size) => Some(*size),
            _ => None,
        })
    }

    /// All `mach(...)` hardware expressions on this entry.
    pub fn mach(&self) -> impl Iterator<Item = &HardwareExpr> {
        self.attributes.iter().filter_map(|a| match a {
            IdbAttribute::Mach(expr) => Some(expr),
            _ => None,
        })
    }

    /// Whether the entry carries a `mach(...)` attribute that could not be
    /// parsed.
    ///
    /// Such entries must never be treated as mach-less fallbacks during
    /// selection; their hardware applicability is simply unknown.
    pub fn has_unparsed_mach(&self) -> bool {
        self.attributes
            .iter()
            .any(|a| matches!(a, IdbAttribute::MachUnparsed { .. }))
    }

    /// `symval(...)` symlink target, if present.
    pub fn symlink_target(&self) -> Option<&str> {
        self.attributes.iter().find_map(|a| match a {
            IdbAttribute::SymlinkValue(target) => Some(target.as_str()),
            _ => None,
        })
    }

    /// `dev(major minor)` numbers, if present.
    pub fn device(&self) -> Option<(u32, u32)> {
        self.attributes.iter().find_map(|a| match a {
            IdbAttribute::Device { major, minor } => Some((*major, *minor)),
            _ => None,
        })
    }

    /// `config(...)` mode, if present.
    pub fn config_mode(&self) -> Option<ConfigMode> {
        self.attributes.iter().find_map(|a| match a {
            IdbAttribute::Config(mode) => Some(mode.clone()),
            _ => None,
        })
    }

    /// The size of the payload as stored in the image archive.
    ///
    /// SGI stores a file compressed when `cmpsize` is non-zero, and
    /// uncompressed otherwise; an entry with `cmpsize(0)` still occupies
    /// `size` bytes in the archive.
    pub fn encoded_size(&self) -> Option<u64> {
        match self.compressed_size() {
            Some(cmpsize) if cmpsize > 0 => Some(cmpsize),
            Some(_) => self.size(),
            None => None,
        }
    }
}
