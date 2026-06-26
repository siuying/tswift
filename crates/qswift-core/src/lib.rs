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
    Arg, BuiltinReceiver, FreeFn, IntrinsicFn, MethodEntry, Outcome, StdContext, StdError,
    StdResult,
};
pub use value::{format_double, EnumObj, IntValue, IntWidth, StructObj, SwiftValue};

/// Register a minimal `print` native for unit tests inside this crate.
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
}
