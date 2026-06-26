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
mod conformance;
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
    conformance::install(interp);
    dictionary::install(interp);
    scalar::install(interp);
    range::install(interp);
    optional::install(interp);
    sequence::install(interp);
    set::install(interp);
    string::install(interp);
}

/// Every standard-library entry registered by [`install`], as semantic coverage
/// keys (`print`, `Array.append`, `Optional.map`, `Sequence.map`, …).
///
/// Authoritative: it installs into a throwaway interpreter and reads the live
/// registry, so it can never drift from the registration code. This is a pure
/// read — the coverage tooling regenerates its inputs from it (see
/// `tools/stdlib-inventory/coverage.py`) rather than from a checked-in copy.
pub fn registered_keys() -> Vec<String> {
    let mut sink = std::io::sink();
    let mut interp = Interpreter::new(&mut sink);
    install(&mut interp);
    interp.registered_keys()
}

#[cfg(test)]
mod tests {
    use super::registered_keys;

    /// The registry exposes a non-empty, sorted, de-duplicated semantic key set.
    #[test]
    fn registered_keys_are_sorted_and_unique() {
        let keys = registered_keys();
        assert!(!keys.is_empty(), "registry should not be empty");

        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "keys should come back sorted");

        let mut deduped = keys.clone();
        deduped.dedup();
        assert_eq!(keys, deduped, "keys should be unique");
    }

    /// `Optional.map`/`flatMap` surface as semantic keys, not the scalar
    /// receiver-dispatch keys (`Int.map`, `Bool.flatMap`, …) they register on.
    #[test]
    fn optional_keys_are_semantic_not_receiver() {
        let keys = registered_keys();
        assert!(keys.iter().any(|k| k == "Optional.map"));
        assert!(keys.iter().any(|k| k == "Optional.flatMap"));
        for leaked in ["Int.map", "Double.map", "Bool.flatMap", "String.map"] {
            assert!(
                !keys.iter().any(|k| k == leaked),
                "receiver-dispatch key `{leaked}` leaked into coverage keys"
            );
        }
    }
}
