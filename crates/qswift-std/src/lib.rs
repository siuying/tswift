//! qswift-std — native standard-library builtins.
//!
//! Every builtin plugs into the interpreter through the [`StdContext`] seam
//! defined in `qswift-core` (see `docs/plan/stdlib-support.md`). Two layers:
//!
//! * **free functions** (`print`, …) registered by name; and
//! * **method intrinsics** registered against a [`BuiltinReceiver`] +
//!   method-name key, each carrying a `mutating` flag.
//!
//! [`install`] wires every builtin into an [`Interpreter`] in one call.

mod array;
mod dictionary;
mod free;
mod optional;
mod range;
mod scalar;
mod sequence;
mod set;
mod string;

use qswift_core::Interpreter;

/// Register every standard-library native into `interp`.
pub fn install(interp: &mut Interpreter<'_>) {
    free::install(interp);
    array::install(interp);
    dictionary::install(interp);
    scalar::install(interp);
    range::install(interp);
    optional::install(interp);
    sequence::install(interp);
    set::install(interp);
    string::install(interp);
}
