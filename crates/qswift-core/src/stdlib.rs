//! The standard-library dispatch seam.
//!
//! `qswift-std` plugs native Swift builtins into the interpreter through the
//! types here. Two layers cooperate (see `docs/plan/stdlib-support.md` §4.1):
//!
//! 1. an **intrinsic registry** keyed by `(BuiltinReceiver, method-name)` for
//!    type-specific members (`Array.append`, `String.uppercased`, …);
//! 2. an **algorithm layer** for `Sequence`/`Collection` methods written once
//!    against an iterator adapter and shared by every builtin sequence.
//!
//! Intrinsics never see the whole [`crate::Interpreter`]. They receive a narrow
//! capability handle, [`StdContext`], that exposes only what a builtin needs:
//! calling a closure, throwing, and writing output. This keeps `qswift-std`
//! decoupled and unit-testable against a mock context.

use std::io::Write;

use crate::value::SwiftValue;
use crate::EvalError;

/// A failure raised by a standard-library intrinsic.
///
/// Mirrors the interpreter's control-flow channel, but exposes only the two
/// outcomes a builtin can produce: a thrown Swift error or a genuine
/// interpreter error. Loop/`return` control flow never crosses the seam.
#[derive(Debug, Clone)]
pub enum StdError {
    /// A thrown Swift error value (Swift's `throw`).
    Throw(SwiftValue),
    /// A genuine interpreter error (trap, type mismatch, …).
    Error(EvalError),
}

impl From<EvalError> for StdError {
    fn from(e: EvalError) -> Self {
        StdError::Error(e)
    }
}

/// The result of an intrinsic: a produced value or a [`StdError`].
pub type StdResult = Result<SwiftValue, StdError>;

/// The narrow capability handle handed to every standard-library intrinsic.
///
/// Defined in core and implemented for [`crate::Interpreter`]; widen only as a
/// concrete slice needs it.
pub trait StdContext {
    /// Call the closure with table id `id`, returning its result.
    fn call_closure(&mut self, id: usize, args: Vec<SwiftValue>) -> StdResult;
    /// The program output sink (`print` and friends write here).
    fn out(&mut self) -> &mut dyn Write;
    /// Build a thrown-error outcome from a Swift error value.
    fn throw(&self, error: SwiftValue) -> StdError {
        StdError::Throw(error)
    }
}

/// The builtin receiver an intrinsic is registered against.
///
/// Grown one slice at a time; new container/scalar kinds are added as their
/// stdlib slice lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinReceiver {
    Array,
    Dictionary,
    Set,
    String,
    Character,
    Int,
    Double,
    Bool,
    Optional,
    Range,
}

impl BuiltinReceiver {
    /// The receiver classification for a runtime value, if it is a builtin one.
    pub fn of(value: &SwiftValue) -> Option<BuiltinReceiver> {
        Some(match value {
            SwiftValue::Array(_) => BuiltinReceiver::Array,
            SwiftValue::Dict(_) => BuiltinReceiver::Dictionary,
            SwiftValue::Str(_) => BuiltinReceiver::String,
            SwiftValue::Int(_) => BuiltinReceiver::Int,
            SwiftValue::Double(_) => BuiltinReceiver::Double,
            SwiftValue::Bool(_) => BuiltinReceiver::Bool,
            SwiftValue::Range { .. } => BuiltinReceiver::Range,
            SwiftValue::Nil => BuiltinReceiver::Optional,
            _ => return None,
        })
    }
}

/// The outcome of an intrinsic call: the call's result value plus the receiver
/// after the call.
///
/// Mutating intrinsics take the receiver by value and return the updated
/// receiver in `receiver`; the dispatcher writes it back to the caller's
/// storage. Non-mutating intrinsics echo the receiver back unchanged and the
/// dispatcher ignores it. By-value (rather than `&mut`) avoids aliasing the
/// `&mut Interpreter` that backs [`StdContext`]; copy-on-write comes for free
/// via `Rc::make_mut`.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub result: SwiftValue,
    pub receiver: SwiftValue,
}

/// A method intrinsic: receives the context, the receiver by value, and the
/// already-evaluated arguments; returns its [`Outcome`].
pub type IntrinsicFn =
    fn(&mut dyn StdContext, SwiftValue, Vec<SwiftValue>) -> Result<Outcome, StdError>;

/// One evaluated free-function argument: its (optional) label and value.
///
/// Free functions see labels because some are label-overloaded
/// (`stride(from:to:by:)` vs `stride(from:through:by:)`).
#[derive(Debug, Clone)]
pub struct Arg {
    pub label: Option<String>,
    pub value: SwiftValue,
}

impl Arg {
    /// A positional (unlabeled) argument.
    pub fn positional(value: SwiftValue) -> Arg {
        Arg { label: None, value }
    }
}

/// A free-function intrinsic (`print`, `min`, `max`, …).
pub type FreeFn = fn(&mut dyn StdContext, Vec<Arg>) -> StdResult;

/// A computed-property intrinsic on a builtin receiver (`Double.isNaN`,
/// `Int.magnitude`, …). Pure: no closures, no mutation, no output.
pub type PropertyFn = fn(SwiftValue) -> StdResult;

/// A `Sequence`/`Collection` algorithm written once against the materialized
/// elements of any builtin sequence receiver (`map`, `filter`, `sorted`, …).
///
/// The dispatcher expands the receiver into its elements via the sequence
/// adapter and passes the (labeled) call arguments through, so closure-taking
/// algorithms can call back into [`StdContext`].
pub type AlgoFn = fn(&mut dyn StdContext, Vec<SwiftValue>, Vec<Arg>) -> StdResult;

/// One registered method intrinsic plus whether it mutates its receiver.
#[derive(Clone, Copy)]
pub struct MethodEntry {
    /// When set, the dispatcher resolves the receiver's lvalue and writes the
    /// returned receiver back after the call.
    pub mutating: bool,
    pub func: IntrinsicFn,
}
