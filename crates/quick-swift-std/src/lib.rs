//! quick-swift-std — native standard-library builtins.
//!
//! msf gives type *shapes*; behaviour lives here. The walking skeleton ships a
//! single builtin, `print`, and a one-call [`install`] that registers it into an
//! [`Interpreter`]. Later milestones add `numeric`/`string`/`collection`/… here.

use std::io::Write;

use quick_swift_core::{Interpreter, SwiftValue};

/// Register every standard-library native into `interp`.
pub fn install(interp: &mut Interpreter<'_, '_>) {
    interp.register_native("print", print);
}

/// Swift's `print(_:separator:terminator:)`, skeleton subset.
///
/// Joins its arguments with a single space and appends a newline — matching
/// `print`'s default `separator: " "` and `terminator: "\n"`.
fn print(out: &mut dyn Write, args: &[SwiftValue]) -> SwiftValue {
    let line = args
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    // The CLI flushes/owns the sink; ignore write errors here to keep the native
    // signature infallible (errors surface when the sink is finalized).
    let _ = writeln!(out, "{line}");
    SwiftValue::Void
}
