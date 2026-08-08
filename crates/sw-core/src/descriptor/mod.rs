//! The product descriptor.
//!
//! Each product ships a descriptor file (e.g. `dist/eoe`) next to its IDB.
//! The descriptor is a *binary* file starting with a `pd001V…` magic and
//! carries the product title, image versions and order, subsystem flags
//! (`required` / `default` / `miniroot` / `patch`) and the dependency rules
//! (`prereq`, `replaces`, `incompat`, `updates`, `follows`).
//!
//! [`model`] defines the complete logical model; the parser currently
//! decodes the header only. The record grammar of the body still needs to
//! be reverse engineered, so undecoded metadata is reported through
//! diagnostics rather than guessed.
pub mod model;
pub(crate) mod parser;
