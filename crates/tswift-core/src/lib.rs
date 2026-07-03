//! tswift-core — the evaluator spine.
//!
//! Language *features* live here (per the implementation plan): the value model,
//! the lexical environment, operator semantics, and the `eval` dispatcher. This
//! milestone covers literals, arithmetic, and `let`/`var` bindings with faithful
//! integer-width overflow/wrap semantics.

pub mod base64;
pub mod decimal;
mod env;
mod fragment_cache;
pub mod grapheme;
mod interp;
pub mod json;
pub mod ops;
pub mod regex;
pub mod result_json;
mod stdlib;
#[cfg(not(target_arch = "wasm32"))]
pub mod suspend;
mod value;

pub use env::{BindError, Binding, Env};
pub use grapheme::graphemes;
pub use interp::{BuiltinParam, EvalError, Interpreter, NativeFn};
pub use regex::{Captures, Regex};
pub use stdlib::{
    collection_range_bounds, materialize_builtin_sequence, scalar_less_than, AlgoFn, Arg,
    BuiltinReceiver, ContextualPropertyFn, FreeFn, IntrinsicFn, LabeledIntrinsicFn,
    LabeledMethodEntry, MethodEntry, Outcome, PropertyFn, PropertySetterFn, StaticFn, StdContext,
    StdError, StdResult, StructMethodFn,
};
pub use value::{
    format_double, format_double_json, EnumObj, IntValue, IntWidth, StructObj, SwiftValue,
};

/// Returns `true` when `s` is a non-empty string that contains no ASCII
/// whitespace characters — the minimum validity contract of Foundation's
/// `URL(string:)` failable initializer.
///
/// Both the URL initializer in `tswift-foundation` and the JSON `URL` decoder
/// in `tswift-core::interp::coding` route through this function so the two
/// remain in sync: a string that `URL(string:)` would reject as `nil` also
/// causes `dataCorrupted` when decoded from JSON.
pub fn is_url_string_valid(s: &str) -> bool {
    !s.is_empty() && !s.bytes().any(|b| b.is_ascii_whitespace())
}

/// Register a minimal stdlib subset for unit tests inside this crate.
///
/// Core has no dependency on `tswift-std`, so its own tests self-provide the
/// few builtins they exercise (`print` and `Array.count`/`isEmpty`), mirroring
/// how `tswift-std::install` populates the real seam.
#[cfg(test)]
pub(crate) fn install_test_print(interp: &mut Interpreter<'_>) {
    use crate::{Arg, BuiltinReceiver, StdContext, StdResult};
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

    // The few sequence algorithms core's own tests exercise. The full layer
    // lives in tswift-std; these tiny copies keep core self-contained.
    fn closure_id(args: &[Arg]) -> Option<usize> {
        args.iter().rev().find_map(|a| match a.value {
            SwiftValue::Closure(id) => Some(id),
            _ => None,
        })
    }
    fn map(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
        let id = closure_id(&args).expect("map closure");
        let mut out = Vec::new();
        for it in items {
            out.push(ctx.call_closure(id, vec![it])?);
        }
        Ok(SwiftValue::Array(std::rc::Rc::new(out)))
    }
    fn filter(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
        let id = closure_id(&args).expect("filter closure");
        let mut out = Vec::new();
        for it in items {
            if ctx
                .call_closure(id, vec![it.clone()])?
                .as_bool()
                .unwrap_or(false)
            {
                out.push(it);
            }
        }
        Ok(SwiftValue::Array(std::rc::Rc::new(out)))
    }
    fn reduce(ctx: &mut dyn StdContext, items: Vec<SwiftValue>, args: Vec<Arg>) -> StdResult {
        let id = closure_id(&args).expect("reduce closure");
        let mut acc = args
            .iter()
            .find(|a| a.label.is_none())
            .map(|a| a.value.clone())
            .expect("reduce initial");
        for it in items {
            acc = ctx.call_closure(id, vec![acc, it])?;
        }
        Ok(acc)
    }
    interp.register_algorithm("map", map);
    interp.register_algorithm("filter", filter);
    interp.register_algorithm("reduce", reduce);
}
