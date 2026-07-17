//! Registry wiring: [`install`] and coverage-key derivation via
//! [`registered_keys`].

use tswift_core::Interpreter;

use crate::{enums, objects, store};

/// Register the currently-supported EventKit surface on `interp`, under the
/// `EventKit` module scope so strict import-gating requires `import EventKit`.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.module("EventKit", |interp| {
        enums::install(interp);
        objects::install(interp);
        store::install(interp);
    });
}

/// Every EventKit entry this crate registers, as coverage keys (`Type.member`,
/// matching `tools/framework-inventory/coverage.py`).
///
/// Builtin enums are invisible to [`Interpreter::registered_keys`] (they resolve
/// through the type table, not a free-fn/intrinsic seam), so the enum case +
/// `init` keys are injected explicitly by [`enums::coverage_keys`] — the same
/// manual-injection pattern `tswift-swiftdata` uses for its `SortOrder` enum and
/// prelude-declared `@Query`.
pub fn registered_keys() -> Vec<String> {
    let mut keys = enums::coverage_keys();
    keys.extend(objects::coverage_keys());
    keys.extend(store::coverage_keys());
    keys.sort();
    keys.dedup();
    keys
}
