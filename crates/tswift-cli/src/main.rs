//! `tswift` — the command-line entry point.
//!
//! Usage:
//!   tswift run <file.swift> [more.swift ...]
//!   tswift dump [--json] <file.swift>
//!
//! `run` analyzes a Swift source file and evaluates it through the tswift
//! runtime, streaming program output to stdout. `dump` prints the typed AST
//! (kind, text, line, resolved type, modifiers) for inspecting how the frontend
//! parses a construct — the fast path when adding a language feature.

mod defaults;
mod httpmock;
mod nethttp;
mod swiftui;

use std::io::{self, Write};
use std::process::ExitCode;

use tswift_core::Interpreter;
use tswift_frontend::{Analysis, AnalyzeError};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();

    match command.as_deref() {
        Some("run") => {
            let rest: Vec<String> = args.collect();
            let allow_network = rest.iter().any(|a| a == "--allow-network");
            let paths: Vec<String> = rest.into_iter().filter(|a| !a.starts_with("--")).collect();
            if paths.is_empty() {
                eprintln!(
                    "error: `run` requires a file path\n\nusage: tswift run [--allow-network] <file.swift> [more.swift ...]"
                );
                ExitCode::FAILURE
            } else {
                run(&paths, allow_network)
            }
        }
        Some("dump") => {
            let rest: Vec<String> = args.collect();
            let json = rest.iter().any(|a| a == "--json");
            match rest.iter().find(|a| !a.starts_with("--")) {
                Some(path) => dump(path, json),
                None => {
                    eprintln!(
                        "error: `dump` requires a file path\n\nusage: tswift dump [--json] <file.swift>"
                    );
                    ExitCode::FAILURE
                }
            }
        }
        Some("swiftui") => swiftui::run(args),
        Some(other) => {
            eprintln!(
                "error: unknown command `{other}`\n\nusage: tswift run <file.swift> | tswift dump [--json] <file.swift> | tswift swiftui render|dispatch <file.swift> [events.json]"
            );
            ExitCode::FAILURE
        }
        None => {
            eprintln!(
                "usage: tswift run <file.swift> [more.swift ...]\n       tswift dump [--json] <file.swift>"
            );
            ExitCode::FAILURE
        }
    }
}

/// Analyze `path` and print its typed AST. Diagnostics (errors/warnings) go to
/// stderr so the tree itself stays clean on stdout.
fn dump(path: &str, json: bool) -> ExitCode {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read `{path}`: {e}");
            return ExitCode::FAILURE;
        }
    };
    let analysis = match analyze_source(&source, path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    for diag in analysis.diagnostics() {
        eprintln!("{}:{}: {}", diag.line, diag.col, diag.message);
    }
    let root = analysis.root();
    if json {
        println!("{}", root.dump_json());
    } else {
        print!("{}", root.dump());
    }
    ExitCode::SUCCESS
}

fn analyze_source(source: &str, filename: &str) -> Result<Analysis, AnalyzeError> {
    Analysis::analyze(source, filename)
}

/// Analyze and evaluate the Swift file(s) at `paths`. Multiple files form one
/// module: their sources are concatenated so cross-file references resolve.
fn run(paths: &[String], allow_network: bool) -> ExitCode {
    let mut source = String::new();
    for path in paths {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                source.push_str(&s);
                source.push('\n');
            }
            Err(e) => {
                eprintln!("error: cannot read `{path}`: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    let path = paths[0].as_str();

    let analysis = match analyze_source(&source, path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Surface diagnostics: warnings (e.g. `#warning`) print and continue;
    // errors (e.g. `#error`, type errors) abort before execution.
    let mut had_error = false;
    for diag in analysis.diagnostics() {
        let kind = if diag.is_error() { "error" } else { "warning" };
        eprintln!(
            "{path}:{}:{}: {kind}: {}",
            diag.line, diag.col, diag.message
        );
        had_error |= diag.is_error();
    }
    if had_error {
        return ExitCode::FAILURE;
    }
    // The interpreter borrows the AST for `'static` (string interpolation leaks
    // small fragment ASTs to match). The process runs one program and exits, so
    // leaking the root analysis here is intentional and bounded.
    let analysis: &'static Analysis = Box::leak(Box::new(analysis));

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let mut interp = Interpreter::new(&mut handle);
    tswift_std::install(&mut interp);
    // The native CLI backs every host service (defaults, file system, database).
    // `UserDefaults` is backed in-process by default (opt into real file
    // persistence with `TSWIFT_DEFAULTS_FILE`; see `defaults.rs`), so the
    // handler must be installed before `install_with` declares the host-fn
    // signatures that route through it.
    interp.set_host_call_handler(std::sync::Arc::new(defaults::DefaultsHandler::new()));
    tswift_foundation::install_with(&mut interp, tswift_core::Capabilities::all());
    interp.set_filename(path);
    // Golden fixtures (and any offline caller) script `URLSession` through a
    // deterministic mock transport instead of the real network; the mock wins
    // over `--allow-network` so tests can never accidentally go online.
    if let Ok(mock_path) = std::env::var("TSWIFT_HTTP_MOCK") {
        match httpmock::load(&mock_path) {
            Ok(transport) => interp.set_http_transport(Box::new(transport)),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else if allow_network {
        interp.set_http_transport(Box::new(nethttp::NetTransport::default()));
    }

    let result = interp.run(analysis);
    let _ = handle.flush();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
