//! # sw-gui-bridge
//!
//! Exposes `sw-core` to the Qt Widgets frontend through a CXX bridge.
//!
//! The boundary is deliberately narrow: the only object crossing it is
//! the opaque `Backend` handle, and results cross as plain data
//! structs. No `sw-core` domain object (`Product`, `Image`,
//! `Subsystem`, `Entry`, ...) is ever handed to C++.
//!
//! * `bridge` declares the CXX ABI itself, in one place.
//! * `backend` implements the opaque `Backend` handle: the loaded
//!   distribution and its object-id table.
//! * `detail` converts `sw-core` domain objects into the detail DTOs
//!   declared by `bridge`.
//! * `entry` converts `sw-core` entries into the entry summary and
//!   detail DTOs declared by `bridge`.
//! * `hardware` extracts hardware attribute candidates from the loaded
//!   distribution and converts `sw-core` entry selections into the
//!   selection DTOs declared by `bridge`.
mod backend;
mod bridge;
mod detail;
mod entry;
mod hardware;
#[cfg(test)]
mod tests;
