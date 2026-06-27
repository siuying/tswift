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
mod bool;
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
    bool::install(interp);
    dictionary::install(interp);
    scalar::install(interp);
    range::install(interp);
    optional::install(interp);
    sequence::install(interp);
    set::install(interp);
    string::install(interp);
}

/// Every standard-library entry registered by [`install`], as coverage keys
/// (`print`, `Array.append`, `Sequence.map`, …).
///
/// Authoritative: it installs into a throwaway interpreter and reads the live
/// registry, so it can never drift from the registration code.
pub fn registered_keys() -> Vec<String> {
    let mut sink = std::io::sink();
    let mut interp = Interpreter::new(&mut sink);
    install(&mut interp);
    interp.registered_keys()
}

#[cfg(test)]
mod coverage_dump {
    /// Dump the live registry keys for coverage tooling. Regenerate with:
    /// `cargo test -p qswift-std dump_registered_keys`.
    #[test]
    fn dump_registered_keys() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let body = super::registered_keys().join("\n") + "\n";
        for relative in [
            "frameworks/stdlib/registered_keys.txt",
            "tools/stdlib-inventory/registered_keys.txt",
        ] {
            std::fs::write(root.join(relative), &body).expect("write registered_keys.txt");
        }
    }
}
