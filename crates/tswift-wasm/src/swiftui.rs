//! Browser bindings for the SwiftUI render host (prototype `swiftui-sandbox`).
//!
//! Where [`crate::run_swift`] compiles + runs a single Swift program for stdout,
//! these entry points drive the *stateful* SwiftUI loop: `swiftUICompile`
//! analyzes a program, finds its root `View`, instantiates a [`Session`] (whose
//! `@State` survives across renders), renders the initial UIIR tree, and stashes
//! the session; `swiftUIDispatch` routes a host event (a button `tap`, a control
//! `set`) into that live session and returns the re-rendered tree.
//!
//! State is kept in a `thread_local` because wasm is single-threaded and a
//! `Session` borrows a leaked `'static` interpreter (mirroring the `Box::leak`
//! pattern the stateless entry point already uses for `Analysis`). Recompiling
//! replaces the session; the previous interpreter's *allocation* is
//! intentionally leaked (acceptable for a throwaway browser sandbox), but its
//! [`Interpreter::teardown`] finalizers are run first via [`clear_session`] so
//! framework-held native resources — notably open SwiftData database handles —
//! are released at replacement instead of accumulating. The only remaining leak
//! is on page unload (no session-replacement event fires), where the browser
//! reclaims the whole wasm instance anyway.

use std::cell::RefCell;

use tswift_core::json::{self, Json};
use tswift_core::{Interpreter, SwiftValue};
use tswift_swiftui::diff;
use tswift_swiftui::session::{Event, Session};
use tswift_swiftui::{find_render_entry, uiir, RenderEntry, PRELUDE};
use wasm_bindgen::prelude::*;

use crate::install_panic_hook;
use tswift_core::result_json::escape as escape_json;

thread_local! {
    /// The live render session for the most recently compiled program. Replaced
    /// on every `swiftUICompile`; read+mutated on every `swiftUIDispatch`.
    static SESSION: RefCell<Option<Session<'static, 'static>>> = const { RefCell::new(None) };
}

/// Compile a SwiftUI program, render its root `View`, and start an interactive
/// session. Returns a JSON envelope:
/// `{"ok":bool,"root":string|null,"tree":<uiir>|null,"error":string|null}`.
#[wasm_bindgen(js_name = swiftUICompile)]
pub fn swiftui_compile(source: &str) -> String {
    install_panic_hook();
    // Drop any prior session first so a failed recompile leaves no stale tree
    // for `swiftUIDispatch` to mutate, running its teardown finalizers so the
    // leaked interpreter's native resources (db handles) are released.
    clear_session();
    compile_impl(source, crate::analysis_cache::swiftui_single_key(source))
}

/// Compile a multi-file SwiftUI module, render its root `View`, and start an
/// interactive session. `module_json` is `{"files":[{"path":"…","contents":"…"},…]}`.
/// Returns the same JSON envelope as `swiftUICompile`.
#[wasm_bindgen(js_name = swiftUICompileModule)]
pub fn swiftui_compile_module(module_json: &str) -> String {
    install_panic_hook();
    clear_session();
    let module = match crate::parse_module(module_json) {
        Ok(m) => m,
        Err(e) => return compile_error(&e),
    };
    // Key on the module's *structural* file boundaries (ordered `(path,
    // contents)` pairs + a multi-file entry-mode tag), NOT on the merged
    // source: a module `[a, b]` must never share a cache entry with a single
    // source equal to `a + b`. `merge()` is only for the analysis input.
    let cache_key = crate::analysis_cache::swiftui_program_key(&module.files);
    let (source, _filename) = module.merge();
    compile_impl(&source, cache_key)
}

/// Route a host event into the live session and return a **patch stream** that
/// the `<swiftui-canvas>` host applies in place (preserving focus, an in-flight
/// slider drag, scroll position, &c.): `{"ok":bool,"patches":[…]|null,
/// "error":string|null}`. `value` is a JSON scalar (a control's new value) or
/// `""` for events without a payload (a tap).
#[wasm_bindgen(js_name = swiftUIDispatch)]
pub fn swiftui_dispatch(id: &str, event: &str, value: &str) -> String {
    install_panic_hook();
    let payload = parse_payload(value);
    SESSION.with(|s| {
        let mut guard = s.borrow_mut();
        let Some(session) = guard.as_mut() else {
            return dispatch_error("no active SwiftUI session — compile first");
        };
        // Snapshot the tree before the event so we can diff it against the
        // re-rendered tree and emit a minimal patch stream (the same engine the
        // `tswift swiftui dispatch` CLI uses).
        // Defensive: `SESSION` is only populated after `compile_impl` renders,
        // so a session here has always rendered at least once.
        let Some(before) = session.current_tree().cloned() else {
            return dispatch_error("session has not rendered yet");
        };
        let ev = Event {
            id: id.to_string(),
            event: event.to_string(),
            value: payload,
        };
        match session.dispatch(&ev) {
            Ok(after) => {
                let patches = diff::diff(&before, &after);
                format!(
                    "{{\"ok\":true,\"patches\":{},\"error\":null}}",
                    diff::to_json(&patches)
                )
            }
            Err(error) => dispatch_error(&error.to_string()),
        }
    })
}

/// Clear the live session, running the outgoing interpreter's teardown
/// finalizers before its (leaked) allocation is discarded so framework-held
/// native resources — open SwiftData database handles — are released at
/// replacement rather than accumulating across recompiles.
fn clear_session() {
    SESSION.with(|s| {
        if let Some(mut old) = s.borrow_mut().take() {
            old.teardown();
        }
    });
}

fn compile_impl(source: &str, cache_key: String) -> String {
    // Prepend the SwiftUI token prelude, the SwiftData `@Query` prelude
    // (ADR-0016 Slice 10b), and the Charts prelude (PlottableValue.value for
    // leading-dot `.value(...)` on mark args). `@Query` degrades to `[]`
    // when the host doesn't back `tswift.db`.
    let program = format!(
        "{PRELUDE}\n{}\n{}\n{source}",
        tswift_swiftdata::QUERY_PRELUDE,
        tswift_charts::PRELUDE,
    );
    // Warm-start cache: a re-submitted byte-identical program (Studio re-run,
    // embed refresh) reuses the prior `Analysis` instead of re-lexing/parsing/
    // analyzing. The interpreter below is still built + run fresh, so this is
    // pure runtime caching (not compilation) and invisible except as latency.
    // `cache_key` carries the caller's structural file boundaries + entry mode
    // (single vs multi-file); the merged `program` is only the analysis input.
    let analysis = match crate::analysis_cache::analyze_keyed(cache_key, || {
        tswift_frontend::Analysis::analyze(&program, "main.swift")
    }) {
        Ok(analysis) => analysis,
        Err(error) => return compile_error(&error.to_string()),
    };

    // Surface parse/sema diagnostics as a compile failure rather than running a
    // half-analyzed tree.
    let mut diagnostics = String::new();
    for diagnostic in analysis.diagnostics() {
        diagnostics.push_str(&format!(
            "{}:{}: {}\n",
            diagnostic.line, diagnostic.col, diagnostic.message
        ));
    }
    if !diagnostics.is_empty() {
        return compile_error(diagnostics.trim_end());
    }

    let Some(entry) = find_render_entry(&analysis) else {
        return compile_error("no `View`- or `App`-conforming struct found");
    };

    // A leaked sink: the session (and its interpreter) outlives this call, and
    // SwiftUI bodies are pure view builders — stdout is not surfaced here.
    let out: &'static mut std::io::Sink = Box::leak(Box::new(std::io::sink()));
    let mut interp = Interpreter::new(out);
    tswift_std::install(&mut interp);
    // The default host-call handler MUST be installed before the Foundation /
    // SwiftData installs: `HostBridge::register` resolves a `None` handler
    // against the default handler *at registration time*, so registering
    // `tswift.db.*` (or `tswift.defaults.*`/`tswift.fs.*`) before a default
    // handler exists silently fails the registration and `is_host_fn` then
    // reports `false` — degrading `.modelContainer(for:)`/`@Query` as
    // "SwiftData is unavailable" even when the page declared the service. This
    // mirrors the `run_swift` path (see `lib.rs`).
    crate::platform::install_host_call_handler(&mut interp);
    tswift_foundation::install_with(&mut interp, crate::platform::host_capabilities());
    tswift_swiftdata::install(
        &mut interp,
        crate::platform::host_capabilities().contains(tswift_core::HostService::Database),
    );
    crate::platform::install_http_transport(&mut interp);
    crate::platform::install_registered_host_fns(&mut interp);
    tswift_swiftui::install(&mut interp);
    tswift_charts::install(&mut interp);
    interp.set_filename("main.swift");
    // The session (below) is leaked to `'static` and holds `Node<'static>`
    // cursors into this AST across dispatch calls, so the interpreter must
    // retain the `Rc` for its lifetime; the warm-start cache keeps only its
    // own independent `Rc` and can evict without freeing an AST in use.
    if let Err(error) = interp.run_retaining(analysis) {
        return compile_error(&error.to_string());
    }

    let interp: &'static mut Interpreter<'static> = Box::leak(Box::new(interp));
    let mut session = match match &entry {
        RenderEntry::View(root) => Session::new(interp, root),
        RenderEntry::App(app) => Session::new_app(interp, app),
    } {
        Ok(session) => session,
        Err(error) => return compile_error(&error.to_string()),
    };
    let tree = match session.render() {
        Ok(tree) => tree,
        Err(error) => return compile_error(&error.to_string()),
    };
    let json = uiir::to_json(&tree);
    SESSION.with(|s| *s.borrow_mut() = Some(session));

    format!(
        "{{\"ok\":true,\"root\":\"{}\",\"tree\":{},\"error\":null}}",
        escape_json(entry.type_name()),
        json
    )
}

fn compile_error(message: &str) -> String {
    format!(
        "{{\"ok\":false,\"root\":null,\"tree\":null,\"error\":\"{}\"}}",
        escape_json(message)
    )
}

fn dispatch_error(message: &str) -> String {
    format!(
        "{{\"ok\":false,\"patches\":null,\"error\":\"{}\"}}",
        escape_json(message)
    )
}

/// Parse a host `set` payload (a JSON scalar) into a Swift value. An empty
/// string (a tap and other payload-less events) yields `None`.
fn parse_payload(value: &str) -> Option<SwiftValue> {
    if value.is_empty() {
        return None;
    }
    match json::parse(value).ok()? {
        Json::Bool(b) => Some(SwiftValue::Bool(b)),
        Json::Int(i) => Some(SwiftValue::int(i as i128)),
        Json::Double(d) => Some(SwiftValue::Double(d)),
        Json::Str(s) => Some(SwiftValue::Str(s)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_field(json: &str) -> bool {
        json.contains("\"ok\":true")
    }

    #[test]
    fn compiles_and_renders_a_counter() {
        let json = swiftui_compile(
            r#"
import SwiftUI
struct CounterView: View {
    @State private var count = 0
    var body: some View {
        VStack {
            Text("\(count)")
            Button("Increment") { count += 1 }
        }
    }
}
"#,
        );
        assert!(ok_field(&json), "json={json}");
        assert!(json.contains("\"root\":\"CounterView\""), "json={json}");
        assert!(json.contains("\"kind\":\"VStack\""), "json={json}");
        assert!(json.contains("\"verbatim\":\"0\""), "json={json}");
    }

    #[test]
    fn dispatch_taps_a_button_and_returns_a_patch_stream() {
        swiftui_compile(
            r#"
import SwiftUI
struct CounterView: View {
    @State private var count = 0
    var body: some View {
        VStack {
            Text("\(count)")
            Button("Increment") { count += 1 }
        }
    }
}
"#,
        );
        // The button is the second child of the root VStack: id "0.1".
        let after = swiftui_dispatch("0.1", "tap", "");
        assert!(ok_field(&after), "after={after}");
        // The only change is the counter Text (id "0.0"), so the diff is a
        // single in-place `setText` — not a full re-mount.
        assert!(
            after.contains(r#"{"op":"setText","id":"0.0","text":"1"}"#),
            "after={after}"
        );
    }

    #[test]
    fn missing_view_is_a_compile_error() {
        let json = swiftui_compile("let x = 1");
        assert!(json.contains("\"ok\":false"), "json={json}");
        assert!(json.contains("View"), "json={json}");
    }

    #[test]
    fn dispatch_without_session_is_an_error() {
        // Clear any session a prior test left behind.
        SESSION.with(|s| *s.borrow_mut() = None);
        let json = swiftui_dispatch("0", "tap", "");
        assert!(json.contains("\"ok\":false"), "json={json}");
        assert!(json.contains("\"patches\":null"), "json={json}");
    }
}
