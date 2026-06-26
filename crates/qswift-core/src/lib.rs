//! qswift-core — the evaluator spine.
//!
//! Language *features* live here (per the implementation plan): the value model,
//! the lexical environment, operator semantics, and the `eval` dispatcher. This
//! milestone covers literals, arithmetic, and `let`/`var` bindings with faithful
//! integer-width overflow/wrap semantics.

mod env;
mod interp;
mod json;
mod ops;
mod stdlib;
pub mod suspend;
mod value;

pub use env::{BindError, Binding, Env};
pub use interp::{EvalError, Interpreter, NativeFn};
pub use stdlib::{
    Arg, BuiltinReceiver, FreeFn, IntrinsicFn, MethodEntry, Outcome, PropertyFn, StdContext,
    StdError, StdResult,
};
pub use value::{format_double, EnumObj, IntValue, IntWidth, StructObj, SwiftValue};

/// Register a minimal stdlib subset for unit tests inside this crate.
///
/// Core has no dependency on `qswift-std`, so its own tests self-provide the
/// few builtins they exercise (`print` and `Array.count`/`isEmpty`), mirroring
/// how `qswift-std::install` populates the real seam.
#[cfg(test)]
pub(crate) fn install_test_print(interp: &mut Interpreter<'_>) {
    use std::io::Write;
    fn print(out: &mut dyn Write, args: &[SwiftValue]) -> SwiftValue {
        let line = args
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(out, "{line}");
        SwiftValue::Void
    }
    interp.register_native("print", print);

    fn array_count(recv: SwiftValue) -> StdResult {
        match recv {
            SwiftValue::Array(v) => Ok(SwiftValue::int(v.len() as i128)),
            _ => Ok(SwiftValue::int(0)),
        }
    }
    fn array_is_empty(recv: SwiftValue) -> StdResult {
        match recv {
            SwiftValue::Array(v) => Ok(SwiftValue::Bool(v.is_empty())),
            _ => Ok(SwiftValue::Bool(true)),
        }
    }
    interp.register_property(BuiltinReceiver::Array, "count", array_count);
    interp.register_property(BuiltinReceiver::Array, "isEmpty", array_is_empty);
}
