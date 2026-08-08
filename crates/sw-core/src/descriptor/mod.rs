//! The product descriptor.
//!
//! Each product ships a descriptor file (e.g. `dist/eoe`) next to its IDB.
//! The descriptor is a *binary* file starting with a `pd001V…` magic and
//! carries the product title, image versions and order, subsystem flags
//! (`required` / `default` / `miniroot` / `patch`) and the dependency rules
//! (`prereq`, `replaces`, `incompat`, `updates`, `follows`).
//!
//! The descriptor is the authoritative source of the
//! product → image → subsystem hierarchy; the IDB only contributes file
//! entries. [`model`] defines the physical records (which keep raw values
//! for fields whose semantics are not confirmed) alongside the logical
//! tree types; the parser decodes the binary grammar deterministically.
pub mod model;
pub(crate) mod parser;
