//! `tswift swiftui render|dispatch` — the SwiftUI render-host subcommands.
//!
//! `render <file>` evaluates a `View`'s `body` into canonical UIIR JSON
//! (Layer B). `dispatch <file> <events.json>` replays a scripted event sequence
//! through the session and prints the per-event patch streams (Layer C). Both
//! prepend the SwiftUI token prelude and run fully offline.

use std::process::ExitCode;

use tswift_core::json::{self, Json};
use tswift_core::Interpreter;
use tswift_frontend::{Analysis, AnalyzeError, SourceFile};
use tswift_swiftui::diff::{self, Patch};
use tswift_swiftui::session::{Event, Session};
use tswift_swiftui::{find_render_entry, uiir, RenderEntry, PRELUDE};

/// Dispatch the `swiftui` subcommand family.
pub fn run(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next().as_deref() {
        Some("render") => match args.next() {
            Some(path) => render(&path),
            None => usage_error("`swiftui render` requires a file path"),
        },
        Some("dispatch") => match (args.next(), args.next()) {
            (Some(path), Some(events)) => dispatch(&path, &events),
            _ => usage_error("`swiftui dispatch` requires <file.swift> <events.json>"),
        },
        Some(other) => usage_error(&format!("unknown swiftui command `{other}`")),
        None => usage_error("usage: tswift swiftui render|dispatch <file.swift> [events.json]"),
    }
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("error: {message}");
    ExitCode::FAILURE
}

/// Read `path`, build an interpreter with the SwiftUI runtime installed, run the
/// (prelude-prefixed) program, and return it plus the root `View` type name.
fn prepare(path: &str) -> Result<(Interpreter<'static>, RenderEntry), ExitCode> {
    let user = std::fs::read_to_string(path).map_err(|e| {
        eprintln!("error: cannot read `{path}`: {e}");
        ExitCode::FAILURE
    })?;
    prepare_source(path, &user)
}

/// Run an App-shaped program through the same render-session setup as the
/// SwiftUI subcommands. `tswift run` intentionally emits no UIIR; it verifies
/// the host-owned entry contract and leaves rendering output to the session
/// hosts (`swiftui render`, wasm, or FFI).
pub(crate) fn run_app(files: &[SourceFile], path: &str) -> ExitCode {
    let user = files
        .iter()
        .map(|file| file.source.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let (mut interp, entry) = match prepare_source(path, &user) {
        Ok(prepared) => prepared,
        Err(code) => return code,
    };
    match render_entry(&mut interp, &entry) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn prepare_source(path: &str, user: &str) -> Result<(Interpreter<'static>, RenderEntry), ExitCode> {
    // Prepend the SwiftUI token prelude, the SwiftData `@Query` prelude
    // (ADR-0016 Slice 10b), and the Charts prelude (PlottableValue.value for
    // leading-dot `.value(...)` on mark args).
    let program = format!(
        "{PRELUDE}\n{}\n{}\n{user}",
        tswift_swiftdata::QUERY_PRELUDE,
        tswift_charts::PRELUDE,
    );
    let analysis = analyze(&program).map_err(|e| {
        eprintln!("error: {e}");
        ExitCode::FAILURE
    })?;
    let analysis: &'static Analysis = Box::leak(Box::new(analysis));
    let entry = find_render_entry(analysis).ok_or_else(|| {
        eprintln!("error: no `View`- or `App`-conforming struct found in `{path}`");
        ExitCode::FAILURE
    })?;

    // The session and interpreter outlive this call via a leaked sink (the
    // process renders one program and exits, so the leak is bounded).
    let out: &'static mut std::io::Sink = Box::leak(Box::new(std::io::sink()));
    let mut interp = Interpreter::new(out);
    tswift_std::install(&mut interp);
    // The native CLI backs every host service, including the database, so a
    // rendered `.modelContainer(for:)` / `@Query` works end-to-end against real
    // SQLite (see `main.rs`; the render host installs the same combined handler).
    interp.set_host_call_handler(std::sync::Arc::new(crate::host::CliHostHandler::new()));
    let caps = tswift_core::Capabilities::all();
    tswift_foundation::install_with(&mut interp, caps);
    tswift_swiftdata::install(
        &mut interp,
        caps.contains(tswift_core::HostService::Database),
    );
    tswift_swiftui::install(&mut interp);
    tswift_charts::install(&mut interp);
    if let Err(e) = interp.run(analysis) {
        eprintln!("error: {e}");
        return Err(ExitCode::FAILURE);
    }
    Ok((interp, entry))
}

fn render(path: &str) -> ExitCode {
    let (mut interp, entry) = match prepare(path) {
        Ok(v) => v,
        Err(code) => return code,
    };
    match render_entry(&mut interp, &entry) {
        Ok(tree) => {
            println!("{}", uiir::to_json(&tree));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(path: &str, events_path: &str) -> ExitCode {
    let (mut interp, entry) = match prepare(path) {
        Ok(v) => v,
        Err(code) => return code,
    };
    let events = match read_events(events_path) {
        Ok(events) => events,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut session = match session_for_entry(&mut interp, &entry) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = session.render() {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }

    // One patch stream per event: diff the tree before/after each dispatch.
    let mut streams: Vec<Vec<Patch>> = Vec::new();
    for event in &events {
        let before = match session.current_tree() {
            Some(tree) => tree.clone(),
            None => break,
        };
        let after = match session.dispatch(event) {
            Ok(tree) => tree,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        };
        streams.push(diff::diff(&before, &after));
    }

    // Print a JSON array of patch streams (one per event).
    let mut out = String::from("[");
    for (i, stream) in streams.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&diff::to_json(stream));
    }
    out.push(']');
    println!("{out}");
    ExitCode::SUCCESS
}

/// Render one host entry through the same session construction used by
/// `dispatch`, then discard the transient session after producing its UIIR.
fn render_entry(
    interp: &mut Interpreter<'_>,
    entry: &RenderEntry,
) -> Result<tswift_core::SwiftValue, tswift_core::EvalError> {
    match entry {
        RenderEntry::View(root) => tswift_swiftui::render_root(interp, root),
        RenderEntry::App(app) => {
            let mut session = Session::new_app(interp, app)?;
            session.render()
        }
    }
}

fn session_for_entry<'i, 'w>(
    interp: &'i mut Interpreter<'w>,
    entry: &RenderEntry,
) -> Result<Session<'i, 'w>, tswift_core::EvalError> {
    match entry {
        RenderEntry::View(root) => Session::new(interp, root),
        RenderEntry::App(app) => Session::new_app(interp, app),
    }
}

fn analyze(source: &str) -> Result<Analysis, AnalyzeError> {
    Analysis::analyze(source, "swiftui.swift")
}

/// Parse an events JSON file: `[ {"id":"0.1","event":"tap","value":<json>?}, … ]`.
fn read_events(path: &str) -> Result<Vec<Event>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read `{path}`: {e}"))?;
    let parsed = json::parse(&text).map_err(|e| format!("invalid events JSON: {e}"))?;
    let Json::Array(items) = parsed else {
        return Err("events JSON must be an array".into());
    };
    let mut events = Vec::new();
    for item in items {
        let id = match item.get("id") {
            Some(Json::Str(s)) => s.clone(),
            _ => return Err("each event needs a string `id`".into()),
        };
        let event = match item.get("event") {
            Some(Json::Str(s)) => s.clone(),
            _ => return Err("each event needs a string `event`".into()),
        };
        let value = item.get("value").and_then(json_to_value);
        events.push(Event { id, event, value });
    }
    Ok(events)
}

/// Map a JSON scalar to a Swift value for an event payload (best effort).
fn json_to_value(json: &Json) -> Option<tswift_core::SwiftValue> {
    use tswift_core::SwiftValue;
    match json {
        Json::Null => None,
        Json::Bool(b) => Some(SwiftValue::Bool(*b)),
        Json::Int(i) => Some(SwiftValue::int(*i as i128)),
        Json::Double(d) => Some(SwiftValue::Double(*d)),
        Json::Str(s) => Some(SwiftValue::Str(s.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{analyze, find_render_entry, RenderEntry};

    fn root_of(src: &str) -> Option<String> {
        let analysis = analyze(src).expect("analyze");
        match find_render_entry(&analysis) {
            Some(RenderEntry::View(root)) => Some(root),
            _ => None,
        }
    }

    #[test]
    fn single_view_is_the_root() {
        let src = "struct Only: View { var body: some View { Text(\"x\") } }";
        assert_eq!(root_of(src).as_deref(), Some("Only"));
    }

    #[test]
    fn composing_parent_is_chosen_over_parameterised_child() {
        // The child appears first but is constructed inside the parent's body,
        // so the parent (constructed by nobody) is the root.
        let src = "
struct Child: View { let n: Int; var body: some View { Text(\"c\") } }
struct Parent: View { var body: some View { Child(n: 1) } }
";
        assert_eq!(root_of(src).as_deref(), Some("Parent"));
    }

    #[test]
    fn mutual_cycle_falls_back_to_first_view() {
        // Both views are constructed (in each other's body); none is unreferenced,
        // so the first in document order wins.
        let src = "
struct A: View { var body: some View { B() } }
struct B: View { var body: some View { A() } }
";
        assert_eq!(root_of(src).as_deref(), Some("A"));
    }

    #[test]
    fn reference_outside_a_view_body_does_not_hide_the_root() {
        // A top-level construction of the root (e.g. a preview/entry) is *not*
        // a composition site, so the root stays selectable.
        let src = "
struct Screen: View { var body: some View { Text(\"x\") } }
let preview = Screen()
";
        assert_eq!(root_of(src).as_deref(), Some("Screen"));
    }

    #[test]
    fn app_entry_precedes_its_content_view() {
        let analysis = analyze(
            "struct Content: View { var body: some View { Text(\"x\") } }\n@main struct Demo: App { var body: some Scene { WindowGroup { Content() } } }",
        )
        .expect("analyze");
        assert_eq!(
            find_render_entry(&analysis),
            Some(RenderEntry::App("Demo".into()))
        );
    }
}
