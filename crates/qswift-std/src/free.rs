//! Free-function intrinsics (no receiver): `print`, …

use qswift_core::{Interpreter, StdContext, StdResult, SwiftValue};

/// Register the free functions of this slice.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_free_fn("print", print);
}

/// Swift's `print(_:separator:terminator:)`, skeleton subset.
///
/// Joins its arguments with a single space and appends a newline — matching
/// `print`'s default `separator: " "` and `terminator: "\n"`.
fn print(ctx: &mut dyn StdContext, args: Vec<SwiftValue>) -> StdResult {
    let line = args
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    // The CLI owns/flushes the sink; ignore write errors here so the builtin
    // stays infallible (errors surface when the sink is finalized).
    let _ = writeln!(ctx.out(), "{line}");
    Ok(SwiftValue::Void)
}
