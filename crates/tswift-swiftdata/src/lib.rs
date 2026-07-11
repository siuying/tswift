//! tswift-swiftdata — substrate for a future SwiftData implementation.
//!
//! Two layers ship here:
//!
//! - [`db`] — the `tswift.db.*` host-service wire (SQL over the host bridge,
//!   mirroring `tswift.defaults.*`/`tswift.fs.*` — see ADR-0014): the tagged
//!   SQL-value codec and the wire op names/signatures. Host-agnostic.
//! - [`model`] — the Swift-facing SwiftData core surface (`@Model`,
//!   `ModelContainer`, `ModelConfiguration`, `ModelContext`) implemented
//!   natively over that wire. `@Model` is discovered generically from the
//!   user's class declaration (no macro expansion) via
//!   [`tswift_core::StdContext::nominal_type_info`]; there is no
//!   SwiftData-specific knowledge in `tswift-core`.
//!
//! [`install`] declares the wire signatures when the platform backs
//! [`tswift_core::HostService::Database`] and always registers the
//! Swift-facing surface (whose initializer raises a clean capability
//! diagnostic when the database service is absent). This crate lives outside
//! `tswift-core` because SQL/database framework logic does not belong in the
//! generic evaluator spine.
//!
//! See `docs/adr/0015-db-host-service-wire.md` for the full wire contract.

pub mod db;
mod model;

use tswift_core::Interpreter;

/// SwiftData's SwiftUI-facing prelude source (the `@Query` property wrapper).
///
/// Kept separate from the native `install`: a SwiftUI render host prepends this
/// to the user program (after `tswift_swiftui::PRELUDE`) so `@Query` resolves,
/// exactly as the SwiftUI token prelude is prepended today. Not included on the
/// plain `tswift run` path (no rendering, no `@Query`).
///
/// `@Query`'s getter fetches the environment's model context on every read;
/// because a render session re-evaluates `body` on every dispatch event
/// (ADR-0016 Slice 10b), a post-`save()` render reflects the new rows with no
/// change-notification hook. It degrades to an empty array when no
/// `.modelContainer(for:)` is in the environment (via `try?`).
pub const QUERY_PRELUDE: &str = r#"
@propertyWrapper
struct Query<Element> {
    var __descriptor: FetchDescriptor<Element>
    var wrappedValue: [Element] {
        guard let __ctx = try? __tswiftCurrentModelContext() else { return [] }
        return (try? __ctx.fetch(__descriptor)) ?? []
    }
    init() {
        __descriptor = FetchDescriptor<Element>()
    }
    init(_ descriptor: FetchDescriptor<Element>) {
        __descriptor = descriptor
    }
    init(sort keyPath: Any, order: SortOrder = .forward) {
        __descriptor = FetchDescriptor<Element>(sortBy: [SortDescriptor(keyPath, order: order)])
    }
    init(filter predicate: Predicate<Element>) {
        __descriptor = FetchDescriptor<Element>(predicate: predicate)
    }
    init(filter predicate: Predicate<Element>, sort keyPath: Any, order: SortOrder = .forward) {
        __descriptor = FetchDescriptor<Element>(
            predicate: predicate,
            sortBy: [SortDescriptor(keyPath, order: order)])
    }
}
"#;

/// Every SwiftData entry this crate registers, as coverage keys (see
/// `tools/framework-inventory/README.md`). Mirrors
/// `tswift_foundation::registered_keys`'s pattern: `interp.registered_keys()`
/// only reports free-fn/method-intrinsic seams, so a few surfaces registered
/// through other seams are injected manually, with a comment explaining why
/// each one is invisible to the generic introspection.
pub fn registered_keys() -> Vec<String> {
    let mut sink = std::io::sink();
    let mut interp = Interpreter::new(&mut sink);
    install(&mut interp, true);
    let mut keys: Vec<String> = interp
        .registered_keys()
        .into_iter()
        .filter_map(|key| match key.as_str() {
            "ModelContainer" => Some("ModelContainer.init".to_string()),
            "ModelConfiguration" => Some("ModelConfiguration.init".to_string()),
            "ModelContext" => Some("ModelContext.init".to_string()),
            "FetchDescriptor" => Some("FetchDescriptor.init".to_string()),
            "SortDescriptor" => Some("SortDescriptor.init".to_string()),
            other
                if other.starts_with("ModelContainer.")
                    || other.starts_with("ModelConfiguration.")
                    || other.starts_with("ModelContext.")
                    || other.starts_with("FetchDescriptor.")
                    || other.starts_with("SortDescriptor.") =>
            {
                Some(other.to_string())
            }
            // Internal plumbing (__tswiftCurrentModelContext) and the
            // SortOrder builtin enum (register_builtin_enum does not add
            // qualified `.forward`/`.reverse` keys to the registry) carry no
            // coverage meaning; excluded by falling through to `_`.
            _ => None,
        })
        .collect();
    // `.modelContainer(for:)` is a generic struct-method modifier
    // (Interpreter::register_struct_method), a seam `registered_keys()` does
    // not introspect (SwiftUI view modifiers are collected separately by
    // that crate). Injected manually so coverage sees it.
    keys.push("View.modelContainer".to_string());
    // `Query` (the `@Query` property wrapper) is declared in Swift source
    // (`QUERY_PRELUDE`), not registered through any Rust seam at all — same
    // pattern as SwiftUI's own `@State`/`@Binding` prelude wrappers. Injected
    // manually; kept in sync by hand with `QUERY_PRELUDE`.
    keys.push("Query.init".to_string());
    keys.push("Query.wrappedValue".to_string());
    keys.sort();
    keys.dedup();
    keys
}

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
    if available {
        for signature_json in db::HOST_FN_SIGNATURES {
            let _ = interp.register_host_fn(signature_json, None);
        }
    }
    // Always register the Swift-facing surface; `ModelContainer(for:)` raises a
    // catchable diagnostic at call time when the database service is absent.
    model::install(interp);
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

#[cfg(test)]
mod coverage_dump {
    #[test]
    fn dump_registered_keys() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join("frameworks/swiftdata/registered_keys.txt");
        let body = super::registered_keys().join("\n") + "\n";
        std::fs::write(&path, body).expect("write registered_keys.txt");
    }
}
