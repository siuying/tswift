//! quick-swift-core — the evaluator spine.
//!
//! Language *features* live here (per the implementation plan): the value model,
//! the `eval` dispatcher, and the seam for native standard-library functions.
//! The walking skeleton implements the thinnest slice: scalar values and a
//! tree-walk that can run an integer-literal call to `print`.

mod interp;
mod value;

pub use interp::{EvalError, Interpreter, NativeFn};
pub use value::SwiftValue;
