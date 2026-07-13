//! tswift-charts — Swift Charts view primitives as runtime builtins.
//!
//! Charts is a **render-host framework** like SwiftUI: `Chart { … }` is a
//! container view and marks (`BarMark`, …) are content-builder children that
//! become view values in the **same** UIIR tree SwiftUI produces. Hosts that
//! already render SwiftUI UIIR can later special-case `kind: "Chart"` /
//! `"BarMark"` without a separate IR. See `notes.md` (Charts autoloop).
//!
//! This crate mirrors the `tswift-swiftui` registry seam: [`install`] wires
//! constructors into an interpreter, and [`registered_keys`] exposes the live
//! registry to the framework-inventory coverage tooling.

mod axis;
mod marks;
mod modifiers;
mod prelude;
mod registry;

#[cfg(test)]
mod tests;

pub use prelude::PRELUDE;
pub use registry::{install, registered_keys};
