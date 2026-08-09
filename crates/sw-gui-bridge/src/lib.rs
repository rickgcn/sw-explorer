//! # sw-gui-bridge
//!
//! Exposes `sw-core` to the Qt Widgets frontend through a CXX bridge.
//!
//! The boundary is deliberately narrow: the only object crossing it is
//! the opaque [`bridge::Backend`] handle, and results cross as plain
//! data structs. No `sw-core` domain object (`Product`, `Image`,
//! `Subsystem`, `Entry`, ...) is ever handed to C++.
pub mod bridge;
