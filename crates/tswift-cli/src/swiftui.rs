//! `tswift swiftui render|dispatch` — the SwiftUI render-host subcommands.
//!
//! `render <file>` evaluates a `View`'s `body` into canonical UIIR JSON
//! (Layer B). `dispatch <file> <events.json>` replays a scripted event sequence
//! through the session and prints the per-event patch streams (Layer C). Both
//! prepend the SwiftUI token prelude and run fully offline.

use std::process::ExitCode;

use tswift_core::json::{self, Json};
use tswift_core::Interpreter;
use tswift_frontend::{Analysis, AnalyzeError, NodeKind};
use tswift_swiftui::diff::{self, Patch};
use tswift_swiftui::session::{Event, Session};
use tswift_swiftui::{uiir, PRELUDE};

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
fn prepare(path: &str) -> Result<(Interpreter<'static>, String), ExitCode> {
    let user = std::fs::read_to_string(path).map_err(|e| {
        eprintln!("error: cannot read `{path}`: {e}");
        ExitCode::FAILURE
    })?;
    let program = format!("{PRELUDE}\n{user}");
    let analysis = analyze(&program).map_err(|e| {
        eprintln!("error: {e}");
        ExitCode::FAILURE
    })?;
    let analysis: &'static Analysis = Box::leak(Box::new(analysis));
    let root_type = find_root_view(analysis).ok_or_else(|| {
        eprintln!("error: no `View`-conforming struct found in `{path}`");
        ExitCode::FAILURE
    })?;

    // The session and interpreter outlive this call via a leaked sink (the
    // process renders one program and exits, so the leak is bounded).
    let out: &'static mut std::io::Sink = Box::leak(Box::new(std::io::sink()));
    let mut interp = Interpreter::new(out);
    tswift_std::install(&mut interp);
    tswift_foundation::install(&mut interp);
    tswift_swiftui::install(&mut interp);
    if let Err(e) = interp.run(analysis) {
        eprintln!("error: {e}");
        return Err(ExitCode::FAILURE);
    }
    Ok((interp, root_type))
}

fn render(path: &str) -> ExitCode {
    let (mut interp, root_type) = match prepare(path) {
        Ok(v) => v,
        Err(code) => return code,
    };
    match tswift_swiftui::render_root(&mut interp, &root_type) {
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
    let (mut interp, root_type) = match prepare(path) {
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

    let mut session = match Session::new(&mut interp, &root_type) {
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

fn analyze(source: &str) -> Result<Analysis, AnalyzeError> {
    Analysis::analyze(source, "swiftui.swift")
}

/// Find the first struct that conforms to `View` (carries a `TypeRef "View"`).
fn find_root_view(analysis: &Analysis) -> Option<String> {
    fn walk(node: tswift_frontend::Node<'_>) -> Option<String> {
        if node.kind() == NodeKind::StructDecl {
            let conforms_view = node
                .children()
                .any(|c| c.kind() == NodeKind::TypeRef && c.text().as_deref() == Some("View"));
            if conforms_view {
                if let Some(name) = node.text() {
                    return Some(name);
                }
            }
        }
        for child in node.children() {
            if let Some(found) = walk(child) {
                return Some(found);
            }
        }
        None
    }
    walk(analysis.root())
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
