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
use tswift_frontend::{Analysis, Severity};
use tswift_swiftui::diff;
use tswift_swiftui::session::{Event, Session};
use tswift_swiftui::{find_render_entry, uiir, RenderEntry, PRELUDE};

use tswift_core::result_json::escape as escape_json;

/// The number of program lines the [`PRELUDE`] occupies once `compile`/`diagnose`
/// splice it ahead of user source as `"{PRELUDE}\n{source}"`. User line *L*
/// (1-based) lands on program line `PRELUDE_LINE_OFFSET + L`; a diagnostic at
/// program line *P > PRELUDE_LINE_OFFSET* maps back to user line
/// `P - PRELUDE_LINE_OFFSET`. Anything at/under the offset is inside the prelude
/// (an internal issue, never the user's) and is dropped.
fn prelude_line_offset() -> u32 {
    // newlines within PRELUDE, plus the one joining `\n` in `"{PRELUDE}\n…"`.
    PRELUDE.matches('\n').count() as u32 + 1
}

/// Lint `source` (spliced after the SwiftUI prelude so its symbols resolve) and
/// return frontend diagnostics as JSON, **without** rendering — the iOS editor's
/// live error-feedback channel, mirroring `tswift-wasm`'s `swiftDiagnostics`.
///
/// Shape: `{"ok":bool,"diagnostics":[{"line":u32,"col":u32,"message":string,
/// "severity":"error"|"warning"}]}`, with `line` mapped back to the user's source
/// (prelude-internal diagnostics are dropped). `ok` is false iff a user-region
/// error is present.
pub(crate) fn diagnose(source: &str) -> String {
    diagnose_named(source, "main.swift")
}

fn diagnose_named(source: &str, filename: &str) -> String {
    let program = format!("{PRELUDE}\n{source}");
    let offset = prelude_line_offset();
    let analysis = match Analysis::analyze(&program, filename) {
        Ok(analysis) => analysis,
        Err(error) => {
            return diagnostics_json(false, &[diagnostic_json(1, 1, "error", &error.to_string())]);
        }
    };

    let mut items = Vec::new();
    let mut had_error = false;
    for diagnostic in analysis.diagnostics() {
        // Drop diagnostics that fall inside the spliced prelude; only the user's
        // own region is actionable in the editor.
        if diagnostic.line <= offset {
            continue;
        }
        let severity = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        had_error |= diagnostic.is_error();
        items.push(diagnostic_json(
            diagnostic.line - offset,
            diagnostic.col,
            severity,
            &diagnostic.message,
        ));
    }
    diagnostics_json(!had_error, &items)
}

fn diagnostic_json(line: u32, col: u32, severity: &str, message: &str) -> String {
    format!(
        "{{\"line\":{line},\"col\":{col},\"severity\":\"{severity}\",\"message\":\"{}\"}}",
        escape_json(message)
    )
}

fn diagnostics_json(ok: bool, items: &[String]) -> String {
    format!("{{\"ok\":{ok},\"diagnostics\":[{}]}}", items.join(","))
}

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
/// Compile without a network transport (offline). Retained for tests; the FFI
/// entrypoint uses [`compile_with_transport`] to carry the host's handler.
#[cfg(test)]
pub(crate) fn compile(slot: &mut Option<SwiftUiSession>, source: &str) -> String {
    compile_with_transport(
        slot,
        source,
        None,
        None,
        &[],
        tswift_core::Capabilities::none(),
    )
}

/// Like [`compile`], but installs the host's `URLSession` transport into the
/// session interpreter so networked views (e.g. a `.task { await URLSession… }`
/// fetch) can resolve requests. Used by the FFI entrypoint, which has the
/// context's HTTP handler; the plain [`compile`] keeps a no-transport session
/// for offline callers and tests.
pub(crate) fn compile_with_transport(
    slot: &mut Option<SwiftUiSession>,
    source: &str,
    http: Option<crate::http::HostHttpHandler>,
    stream_http: Option<crate::http::StreamingHandlerConfig>,
    host_fns: &[crate::host::HostFnRegistration],
    caps: tswift_core::Capabilities,
) -> String {
    // Drop any prior session first, so a failed recompile leaves no stale tree.
    *slot = None;
    match build(source, "main.swift", http, stream_http, host_fns, caps) {
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
fn build(
    source: &str,
    filename: &str,
    http: Option<crate::http::HostHttpHandler>,
    stream_http: Option<crate::http::StreamingHandlerConfig>,
    host_fns: &[crate::host::HostFnRegistration],
    caps: tswift_core::Capabilities,
) -> Result<(SwiftUiSession, String, String), String> {
    let program = format!("{PRELUDE}\n{source}");
    let analysis = Analysis::analyze(&program, filename).map_err(|e| e.to_string())?;

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

    let entry =
        find_render_entry(&analysis).ok_or("no `View`- or `App`-conforming struct found")?;

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
    // Route SwiftUI's Foundation install through the same capability-aware path
    // as the one-shot run: host-backed APIs gate on the services the embedding
    // explicitly declared (`tswift_declare_host_service`), not implicit full caps.
    tswift_foundation::install_with(&mut interp, caps);
    tswift_swiftdata::install(
        &mut interp,
        caps.contains(tswift_core::HostService::Database),
    );
    tswift_swiftui::install(&mut interp);
    interp.set_filename(filename);
    // Wire the host's URLSession transport (if any) so scripts' network calls —
    // including those fired from `.task {}` via `run_mount_tasks` — resolve.
    // Streaming config wins when both are present, matching the `run` path.
    if let Some(config) = stream_http {
        interp.set_http_transport(Box::new(crate::http::StreamingHostHttpHandler::from(
            config,
        )));
    } else if let Some(handler) = http {
        interp.set_http_transport(Box::new(handler));
    }
    crate::host::install(&mut interp, host_fns);
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

    let mut session = match match &entry {
        RenderEntry::View(root) => Session::new(interp_ref, root),
        RenderEntry::App(app) => Session::new_app(interp_ref, app),
    } {
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
    Ok((bundle, tree_json, entry.type_name().to_string()))
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

/// Fire any pending `.task {}` closures on the live session's tree and return a
/// patch stream in the **same** envelope as [`dispatch`]
/// (`{"ok":bool,"patches":[…]|null,"error":string|null}`). The host calls this
/// once after a successful [`compile`] to run appear-time async work. Safe with
/// no `.task` modifiers present — it re-renders and yields an empty patch list.
pub(crate) fn run_mount_tasks(slot: &mut Option<SwiftUiSession>) -> String {
    let Some(bundle) = slot.as_mut() else {
        return dispatch_error_json("no active SwiftUI session — compile first");
    };
    let Some(session) = bundle.session.as_mut() else {
        return dispatch_error_json("session has not rendered yet");
    };
    let Some(before) = session.current_tree().cloned() else {
        return dispatch_error_json("session has not rendered yet");
    };
    match session.run_mount_tasks() {
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

/// Lint a multi-file module (same as [`diagnose`] but accepts a
/// `{"files":[…]}` payload).
pub(crate) fn diagnose_module(module_json: &str) -> String {
    let module = match crate::run::parse_module(module_json) {
        Ok(m) => m,
        Err(e) => {
            return diagnostics_json(false, &[diagnostic_json(1, 1, "error", &e)]);
        }
    };
    let (source, filename) = module.merge();
    diagnose_named(&source, filename)
}

/// Compile a multi-file module into a SwiftUI render session (same as
/// [`compile_with_transport`] but accepts a `{"files":[…]}` payload).
pub(crate) fn compile_module_with_transport(
    slot: &mut Option<SwiftUiSession>,
    module_json: &str,
    http: Option<crate::http::HostHttpHandler>,
    stream_http: Option<crate::http::StreamingHandlerConfig>,
    host_fns: &[crate::host::HostFnRegistration],
    caps: tswift_core::Capabilities,
) -> String {
    let module = match crate::run::parse_module(module_json) {
        Ok(m) => m,
        Err(e) => return compile_error_json(&e),
    };
    let (source, filename) = module.merge();
    *slot = None;
    match build(&source, filename, http, stream_http, host_fns, caps) {
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
    fn compile_then_run_mount_tasks_patches_task_state() {
        let mut slot = None;
        let result = compile(
            &mut slot,
            r#"
import SwiftUI
struct V: View {
    @State private var ready = false
    var body: some View {
        Text(ready ? "ready" : "wait").task { ready = true }
    }
}
"#,
        );
        assert!(result.contains("\"ok\":true"), "{result}");
        assert!(result.contains("wait"), "initial tree shows wait: {result}");
        let patches = run_mount_tasks(&mut slot);
        assert!(patches.contains("\"ok\":true"), "{patches}");
        assert!(patches.contains("ready"), "task updated state: {patches}");
    }

    #[test]
    fn run_mount_tasks_without_session_is_error() {
        let mut slot = None;
        let json = run_mount_tasks(&mut slot);
        assert!(json.contains("\"ok\":false"), "{json}");
        assert!(json.contains("\"patches\":null"), "{json}");
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
