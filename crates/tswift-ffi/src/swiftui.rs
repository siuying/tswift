//! The stateful SwiftUI render session behind `TSwiftUI`.
//!
//! `tswift-wasm`'s `swiftui` entry points keep the live `Session` in a
//! `thread_local` and `Box::leak` the analysis, the stdout sink, and the
//! interpreter — acceptable for a throwaway browser sandbox. The native host
//! instead **owns and reclaims** them: [`SwiftUiSession`] is a hand-rolled
//! self-referential bundle stored in the [`Context`], freed deterministically on
//! its `Drop` (and on recompile). This is the reclaimable model from
//! `docs/plan/native-host.md`; all of its `unsafe` is confined here.
//!
//! [`Context`]: crate::Context

use tswift_core::json::{self, Json};
use tswift_core::{Interpreter, SwiftValue};
use tswift_frontend::Analysis;
use tswift_swiftui::diff;
use tswift_swiftui::session::{Event, Session};
use tswift_swiftui::{find_root_view, uiir, PRELUDE};

use crate::util::escape_json;

/// A live SwiftUI render session plus the heap allocations it borrows.
///
/// The fields form a borrow chain: `session` borrows `*interp`, which borrows
/// `*out` and `*analysis`. We store the borrowees as raw pointers (each a
/// `Box` we `into_raw`'d) and reclaim them in [`Drop`] *after* dropping the
/// session, so no borrower outlives its borrowee.
pub(crate) struct SwiftUiSession {
    /// `None` only transiently during construction/teardown; `Some` once the
    /// initial render succeeds.
    session: Option<Session<'static, 'static>>,
    /// Owned `Box<Interpreter<'static>>`; null until the program has run.
    interp: *mut Interpreter<'static>,
    /// Owned `Box<std::io::Sink>` — the interpreter's (discarded) stdout.
    out: *mut std::io::Sink,
    /// Owned `Box<Analysis>` — the analyzed program the interpreter ran.
    analysis: *mut Analysis,
}

impl Drop for SwiftUiSession {
    fn drop(&mut self) {
        // Drop the session first so its `&'static mut Interpreter` borrow ends
        // before we free the interpreter; then reclaim the owners in dependency
        // order (interp borrows out + analysis, so it goes first).
        self.session = None;
        // SAFETY: each pointer was produced by `Box::into_raw` in `compile` and
        // is freed exactly once here. `interp` may be null (an error before it
        // was created); the others are always live once the bundle exists.
        unsafe {
            if !self.interp.is_null() {
                drop(Box::from_raw(self.interp));
            }
            drop(Box::from_raw(self.out));
            drop(Box::from_raw(self.analysis));
        }
    }
}

/// Compile a SwiftUI program, render its root view, and install the live
/// session into `slot` (replacing any prior one). Returns the UIIR envelope
/// `{"ok":bool,"root":string|null,"tree":<uiir>|null,"error":string|null}`.
pub(crate) fn compile(slot: &mut Option<SwiftUiSession>, source: &str) -> String {
    // Drop any prior session first, so a failed recompile leaves no stale tree.
    *slot = None;
    match build(source) {
        Ok((bundle, tree_json, root)) => {
            *slot = Some(bundle);
            format!(
                "{{\"ok\":true,\"root\":\"{}\",\"tree\":{},\"error\":null}}",
                escape_json(&root),
                tree_json
            )
        }
        Err(message) => compile_error_json(&message),
    }
}

/// Build the reclaimable bundle. On any failure every allocation made so far is
/// freed (via the partially-initialised bundle's `Drop`), so no path leaks.
fn build(source: &str) -> Result<(SwiftUiSession, String, String), String> {
    let program = format!("{PRELUDE}\n{source}");
    let analysis = Analysis::analyze(&program, "main.swift").map_err(|e| e.to_string())?;

    let mut diagnostics = String::new();
    for diagnostic in analysis.diagnostics() {
        diagnostics.push_str(&format!(
            "{}:{}: {}\n",
            diagnostic.line, diagnostic.col, diagnostic.message
        ));
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.trim_end().to_string());
    }

    let root = find_root_view(&analysis).ok_or("no `View`-conforming struct found")?;

    // From here on, allocations are owned by `bundle`; every early return moves
    // `bundle` out so its `Drop` reclaims them.
    let mut bundle = SwiftUiSession {
        session: None,
        interp: std::ptr::null_mut(),
        out: Box::into_raw(Box::new(std::io::sink())),
        analysis: Box::into_raw(Box::new(analysis)),
    };

    // SAFETY: `out`/`analysis` are unique fresh `Box` allocations; we form one
    // borrow of each, valid for the bundle's lifetime (they outlive `interp`,
    // which is freed first in `Drop`). The references never escape the bundle.
    let out_ref: &'static mut std::io::Sink = unsafe { &mut *bundle.out };
    let analysis_ref: &'static Analysis = unsafe { &*bundle.analysis };

    let mut interp = Interpreter::new(out_ref);
    tswift_std::install(&mut interp);
    tswift_foundation::install(&mut interp);
    tswift_swiftui::install(&mut interp);
    interp.set_filename("main.swift");
    if let Err(error) = interp.run(analysis_ref) {
        // Stringify and drop the error *before* tearing down: it may hold the
        // faked-'static refs into `interp`/`analysis`, so its `Display`/`Drop`
        // must not run after those allocations are freed.
        let message = error.to_string();
        drop(error);
        drop(interp);
        drop(bundle);
        return Err(message);
    }

    bundle.interp = Box::into_raw(Box::new(interp));
    // SAFETY: `interp` is a unique fresh `Box`; one `&'static mut` is formed and
    // handed to the session, which is dropped before `interp` is freed.
    let interp_ref: &'static mut Interpreter<'static> = unsafe { &mut *bundle.interp };

    let mut session = match Session::new(interp_ref, &root) {
        Ok(session) => session,
        Err(error) => {
            // `interp_ref` was consumed and dropped by the failed `Session::new`.
            // Stringify + drop the error before `bundle`'s `Drop` frees
            // `interp`/`out`/`analysis` (the error may borrow them).
            let message = error.to_string();
            drop(error);
            drop(bundle);
            return Err(message);
        }
    };
    let tree = match session.render() {
        Ok(tree) => tree,
        Err(error) => {
            // Stringify + drop the error, then end the session's borrow, before
            // `bundle`'s `Drop` frees `interp`.
            let message = error.to_string();
            drop(error);
            drop(session);
            drop(bundle);
            return Err(message);
        }
    };
    let tree_json = uiir::to_json(&tree);
    bundle.session = Some(session);
    Ok((bundle, tree_json, root))
}

/// Route a host event into the live session and return a patch stream:
/// `{"ok":bool,"patches":[…]|null,"error":string|null}`. `event_json` is an
/// object `{"id":string,"event":string,"value"?:scalar}` (value omitted for a
/// `tap`).
pub(crate) fn dispatch(slot: &mut Option<SwiftUiSession>, event_json: &str) -> String {
    let Some(bundle) = slot.as_mut() else {
        return dispatch_error_json("no active SwiftUI session — compile first");
    };
    let Some(session) = bundle.session.as_mut() else {
        return dispatch_error_json("session has not rendered yet");
    };
    let event = match parse_event(event_json) {
        Ok(event) => event,
        Err(message) => return dispatch_error_json(&message),
    };
    let Some(before) = session.current_tree().cloned() else {
        return dispatch_error_json("session has not rendered yet");
    };
    match session.dispatch(&event) {
        Ok(after) => {
            let patches = diff::diff(&before, &after);
            format!(
                "{{\"ok\":true,\"patches\":{},\"error\":null}}",
                diff::to_json(&patches)
            )
        }
        Err(error) => dispatch_error_json(&error.to_string()),
    }
}

/// Parse the `{"id","event","value"?}` envelope into an [`Event`].
fn parse_event(event_json: &str) -> Result<Event, String> {
    let parsed = json::parse(event_json)?;
    let id = match parsed.get("id") {
        Some(Json::Str(s)) => s.clone(),
        _ => return Err("event.id must be a string".to_string()),
    };
    let event = match parsed.get("event") {
        Some(Json::Str(s)) => s.clone(),
        _ => return Err("event.event must be a string".to_string()),
    };
    let value = payload_from_json(parsed.get("value"));
    Ok(Event { id, event, value })
}

/// Convert a JSON scalar payload into a Swift value; non-scalars and absence
/// yield `None` (a payload-less event such as a tap).
fn payload_from_json(value: Option<&Json>) -> Option<SwiftValue> {
    match value? {
        Json::Bool(b) => Some(SwiftValue::Bool(*b)),
        Json::Int(i) => Some(SwiftValue::int(*i as i128)),
        Json::Double(d) => Some(SwiftValue::Double(*d)),
        Json::Str(s) => Some(SwiftValue::Str(s.clone())),
        _ => None,
    }
}

pub(crate) fn compile_error_json(message: &str) -> String {
    format!(
        "{{\"ok\":false,\"root\":null,\"tree\":null,\"error\":\"{}\"}}",
        escape_json(message)
    )
}

pub(crate) fn dispatch_error_json(message: &str) -> String {
    format!(
        "{{\"ok\":false,\"patches\":null,\"error\":\"{}\"}}",
        escape_json(message)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const COUNTER: &str = r#"
struct CounterView: View {
    @State private var count = 0
    var body: some View {
        VStack {
            Text("\(count)")
            Button("Increment") { count += 1 }
        }
    }
}
"#;

    #[test]
    fn compiles_and_renders_a_counter() {
        let mut slot = None;
        let json = compile(&mut slot, COUNTER);
        assert!(json.contains("\"ok\":true"), "{json}");
        assert!(json.contains("\"root\":\"CounterView\""), "{json}");
        assert!(json.contains("\"kind\":\"VStack\""), "{json}");
        assert!(json.contains("\"verbatim\":\"0\""), "{json}");
        assert!(slot.is_some());
    }

    #[test]
    fn dispatch_taps_a_button_and_patches() {
        let mut slot = None;
        compile(&mut slot, COUNTER);
        let after = dispatch(&mut slot, r#"{"id":"0.1","event":"tap"}"#);
        assert!(after.contains("\"ok\":true"), "{after}");
        assert!(
            after.contains(r#"{"op":"setText","id":"0.0","text":"1"}"#),
            "{after}"
        );
    }

    #[test]
    fn missing_view_is_compile_error() {
        let mut slot = None;
        let json = compile(&mut slot, "let x = 1");
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(json.contains("View"), "{json}");
        assert!(slot.is_none());
    }

    #[test]
    fn dispatch_without_session_is_error() {
        let mut slot = None;
        let json = dispatch(&mut slot, r#"{"id":"0","event":"tap"}"#);
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(json.contains("\"patches\":null"), "{json}");
    }

    #[test]
    fn repeated_compile_dispatch_is_stable() {
        // Stress the reclaimable bundle: each recompile drops the previous
        // session (interp + sink + analysis). A double-free/UAF in that path
        // would crash here; a leak would show under a leak-checked run.
        let mut slot = None;
        for _ in 0..50 {
            compile(&mut slot, COUNTER);
            let after = dispatch(&mut slot, r#"{"id":"0.1","event":"tap"}"#);
            assert!(after.contains("\"ok\":true"), "{after}");
        }
        // Dropping `slot` reclaims the final bundle.
        drop(slot);
    }

    #[test]
    fn recompile_replaces_session_without_leaking() {
        let mut slot = None;
        compile(&mut slot, COUNTER);
        // A failed recompile must clear the prior session.
        let json = compile(&mut slot, "let x = 1");
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(slot.is_none());
        // And a fresh successful compile re-establishes one.
        compile(&mut slot, COUNTER);
        assert!(slot.is_some());
    }
}
