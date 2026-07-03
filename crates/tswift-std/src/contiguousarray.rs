//! `ContiguousArray` method intrinsics.
//!
//! `ContiguousArray` is semantically identical to `Array` in this runtime:
//! the value is represented as `SwiftValue::Array`, and all Array methods
//! apply without change.  This module registers the same Array member set
//! under `BuiltinReceiver::ContiguousArray` so that `ContiguousArray.*`
//! keys appear in the stdlib coverage registry.
//!
//! Dispatch at runtime still routes through `BuiltinReceiver::Array` (since
//! `BuiltinReceiver::of(SwiftValue::Array) == Array`), so no new code paths
//! are needed.

use tswift_core::{BuiltinReceiver, Interpreter};

/// Register all `ContiguousArray` intrinsics by mirroring the `Array` set.
pub fn install(interp: &mut Interpreter<'_>) {
    // Re-use the Array registration function pointed at ContiguousArray.
    crate::array::install_for(interp, BuiltinReceiver::ContiguousArray);
}
