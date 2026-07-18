//! tswift-eventkit — a headless, in-memory model of EventKit.
//!
//! EventKit is an Apple-platform-only, Objective-C framework: on device it
//! reads and writes the user's Calendar and Reminders database through the
//! system Calendar daemon behind a permission prompt. There is **no** web/wasm
//! equivalent (no shared calendar store in a browser sandbox), so — like
//! SwiftData's SQLite model (`tswift-swiftdata`) — this crate implements a
//! headless, in-memory model of the store: [`EKEventStore`] backed by in-memory
//! arrays of `EKCalendar` / `EKEvent` / `EKReminder` value objects, permission
//! requests that resolve deterministically (no UI to prompt), and CRUD
//! (`save`/`remove`/`commit`/`reset`) against that store.
//!
//! This crate mirrors the `tswift-swiftdata` / `tswift-charts` registry seam:
//! [`install`] wires constructors + intrinsics into an interpreter, and
//! [`registered_keys`] exposes the live registry to the framework-inventory
//! coverage tooling (`tools/framework-inventory/coverage.py`).
//!
//! See `frameworks/eventkit/scope.toml` for the declared in-scope surface and
//! the documented gaps (NSPredicate queries, change notifications, virtual
//! conference providers, NSError-domain bridging).

mod enums;
mod items;
mod objects;
mod registry;
mod store;

#[cfg(test)]
mod tests;

pub use registry::{install, registered_keys};
