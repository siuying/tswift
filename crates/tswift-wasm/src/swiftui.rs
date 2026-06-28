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
//! replaces the session; the previous interpreter's allocation is intentionally
//! leaked — acceptable for a throwaway browser sandbox.

use std::cell::RefCell;

use tswift_core::json::{self, Json};
use tswift_core::{Interpreter, SwiftValue};
use tswift_frontend::{Analysis, Node, NodeKind};
use tswift_swiftui::diff;
use tswift_swiftui::session::{Event, Session};
use tswift_swiftui::{uiir, PRELUDE};
use wasm_bindgen::prelude::*;

use crate::{escape_json, install_panic_hook};

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
    // for `swiftUIDispatch` to mutate.
    SESSION.with(|s| *s.borrow_mut() = None);
    compile_impl(source)
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

fn compile_impl(source: &str) -> String {
    let program = format!("{PRELUDE}\n{source}");
    let analysis = match Analysis::analyze(&program, "main.swift") {
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

    let analysis: &'static Analysis = Box::leak(Box::new(analysis));
    let Some(root) = find_root_view(analysis) else {
        return compile_error("no `View`-conforming struct found");
    };

    // A leaked sink: the session (and its interpreter) outlives this call, and
    // SwiftUI bodies are pure view builders — stdout is not surfaced here.
    let out: &'static mut std::io::Sink = Box::leak(Box::new(std::io::sink()));
    let mut interp = Interpreter::new(out);
    tswift_std::install(&mut interp);
    tswift_foundation::install(&mut interp);
    tswift_swiftui::install(&mut interp);
    interp.set_filename("main.swift");
    if let Err(error) = interp.run(analysis) {
        return compile_error(&error.to_string());
    }

    let interp: &'static mut Interpreter<'static> = Box::leak(Box::new(interp));
    let mut session = match Session::new(interp, &root) {
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
        escape_json(&root),
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

/// Find the root `View` struct to render: the one no other view *constructs*
/// inside a view body. Ported from `tswift-cli`'s `swiftui` subcommand so the
/// browser host picks the same top-level screen.
fn find_root_view(analysis: &Analysis) -> Option<String> {
    use std::collections::HashSet;
    let mut views: Vec<String> = Vec::new();
    let mut constructed: HashSet<String> = HashSet::new();

    fn callee_name(node: &Node<'_>) -> Option<String> {
        if node.kind() != NodeKind::CallExpr {
            return None;
        }
        let callee = node.children().next()?;
        if callee.kind() == NodeKind::IdentExpr {
            callee.text()
        } else {
            None
        }
    }

    fn walk(
        node: Node<'_>,
        in_view: bool,
        views: &mut Vec<String>,
        constructed: &mut HashSet<String>,
    ) {
        let mut child_in_view = in_view;
        if node.kind() == NodeKind::StructDecl {
            let conforms_view = node
                .children()
                .any(|c| c.kind() == NodeKind::TypeRef && c.text().as_deref() == Some("View"));
            if conforms_view {
                if let Some(name) = node.text() {
                    views.push(name);
                }
                child_in_view = true;
            }
        }
        if in_view {
            if let Some(name) = callee_name(&node) {
                constructed.insert(name);
            }
        }
        for child in node.children() {
            walk(child, child_in_view, views, constructed);
        }
    }

    walk(analysis.root(), false, &mut views, &mut constructed);
    views
        .iter()
        .find(|v| !constructed.contains(*v))
        .or_else(|| views.first())
        .cloned()
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
