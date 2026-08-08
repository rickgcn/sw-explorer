//! # sw-core
//!
//! Reads SGI IRIX software distributions (`dist` directories) as their
//! native model:
//!
//! ```text
//! Distribution
//! └── Product          (descriptor + IDB)
//!     └── Image        (eoe.sw — logical group + physical archive)
//!         └── Subsystem          (smallest installable unit)
//!             └── Entry          (one file / directory / link / ...)
//! ```
//!
//! Typical usage:
//!
//! ```no_run
//! use sw_core::distribution::Distribution;
//! use sw_core::mach::eval::HardwareProfile;
//! use sw_core::query::Query;
//!
//! let dist = Distribution::open("/archive/IRIX/dist")?;
//!
//! let found = dist.find(&Query::path("*ge7.bin"));
//!
//! let profile = HardwareProfile::builder()
//!     .set("CPUBOARD", "IP22")
//!     .set("GFXBOARD", "EXPRESS")
//!     .set("MODE", "32bit")
//!     .build();
//! let selection = dist.select(&profile);
//!
//! let mut reader = dist.image_reader();
//! for entry in selection.selected {
//!     if let Ok(payload) = reader.read(entry) {
//!         let bytes = payload.decode()?;
//!         // ...
//!     }
//! }
//! # Ok::<(), sw_core::error::Error>(())
//! ```
pub mod compress;
pub mod descriptor;
pub mod diagnostic;
pub mod distribution;
pub mod error;
pub mod extract;
pub mod idb;
pub mod image;
pub mod mach;
pub mod names;
pub mod path;
pub mod query;
pub mod selection;
