//! tswift-swiftdata — substrate for a future SwiftData implementation.
//!
//! This slice ships only the `tswift.db.*` host-service wire (SQL over the
//! host bridge, mirroring `tswift.defaults.*`/`tswift.fs.*` — see ADR-0014):
//! [`db`] owns the tagged SQL-value codec and the wire op names/signatures;
//! [`install`] declares those signatures on the interpreter when the
//! platform backs [`tswift_core::HostService::Database`].
//!
//! There is **no Swift-facing `SwiftData`/`@Model` API here yet** — that is
//! future work layered on top of this substrate. This crate intentionally
//! stays framework-agnostic about *SwiftData* semantics (no `@Model`
//! macro-expansion knowledge, no query-builder types) while still living
//! outside `tswift-core` (SQL/database framework logic does not belong in
//! the generic evaluator spine).
//!
//! See `docs/adr/0015-db-host-service-wire.md` for the full wire contract.

pub mod db;

use tswift_core::Interpreter;

#[cfg(test)]
use tswift_core::StdContext;

/// Declare the `tswift.db.*` host-function signatures on `interp` when
/// `available` (the platform backs [`tswift_core::HostService::Database`]).
///
/// Mirrors `tswift_foundation::user_defaults::install`'s pattern: this crate
/// only *declares* the wire signatures; the platform embedding supplies the
/// handler via `Interpreter::set_host_call_handler` (or a per-function
/// handler it cannot reach from here). Registration failure (the embedding
/// declared the service but never installed a call handler) is swallowed for
/// the same reason `tswift-foundation` swallows it: a later
/// `StdContext::is_host_fn` check degrades gracefully instead of panicking
/// an otherwise behaviour-preserving install.
pub fn install(interp: &mut Interpreter<'_>, available: bool) {
    if !available {
        return;
    }
    for signature_json in db::HOST_FN_SIGNATURES {
        let _ = interp.register_host_fn(signature_json, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_core::HostService;

    #[test]
    fn install_registers_every_op_when_available() {
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        interp.set_host_call_handler(std::sync::Arc::new(NoopHandler));
        install(&mut interp, true);
        let ctx: &mut dyn StdContext = &mut interp;
        for op in [
            db::OP_OPEN,
            db::OP_CLOSE,
            db::OP_EXECUTE,
            db::OP_QUERY,
            db::OP_BEGIN,
            db::OP_COMMIT,
            db::OP_ROLLBACK,
        ] {
            assert!(ctx.is_host_fn(op), "{op} not registered");
        }
    }

    #[test]
    fn install_is_a_noop_when_unavailable() {
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        install(&mut interp, false);
        let ctx: &mut dyn StdContext = &mut interp;
        assert!(!ctx.is_host_fn(db::OP_OPEN));
    }

    struct NoopHandler;
    impl tswift_core::HostCallHandler for NoopHandler {
        fn call(&self, _name: &str, _args_json: &str) -> Result<String, String> {
            Ok("null".to_string())
        }
    }

    #[test]
    fn namespace_matches_host_service() {
        for op in db::HOST_FN_SIGNATURES {
            let sig = tswift_core::HostSignature::from_json(op).unwrap();
            assert_eq!(
                HostService::for_function(&sig.name),
                Some(HostService::Database)
            );
        }
    }
}
