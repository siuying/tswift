//! `tswift` — the command-line entry point.
//!
//! Usage:
//!   tswift run <file.swift> [more.swift ...]
//!   tswift run <dir>              (all *.swift in <dir>; main.swift is entry)
//!   tswift dump [--json] <file.swift>
//!
//! `run` analyzes a Swift source file and evaluates it through the tswift
//! runtime, streaming program output to stdout. `dump` prints the typed AST
//! (kind, text, line, resolved type, modifiers) for inspecting how the frontend
//! parses a construct — the fast path when adding a language feature.

mod db;
mod defaults;
mod fs;
mod host;
mod httpmock;
mod nethttp;
mod sqlite_ffi;
mod swiftui;

use std::io::{self, Write};
use std::process::ExitCode;

use tswift_core::Interpreter;
use tswift_frontend::{Analysis, AnalyzeError, SourceFile};

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
                    "error: `run` requires a file or directory path\n\nusage: tswift run [--allow-network] <file.swift> [more.swift ...]\n       tswift run [--allow-network] <dir>"
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
                "usage: tswift run <file.swift> [more.swift ...]\n       tswift run <dir>\n       tswift dump [--json] <file.swift>"
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

/// Expand `run` path arguments into an ordered list of [`SourceFile`]s.
///
/// A single directory argument is expanded to every `*.swift` file it directly
/// contains, sorted for determinism. Explicit file arguments are read in the
/// order given. Directories load `main.swift` as the program entry.
fn collect_source_files(paths: &[String]) -> Result<Vec<SourceFile>, String> {
    // `tswift run <dir>`: expand the directory's `.swift` files.
    if paths.len() == 1 {
        let meta =
            std::fs::metadata(&paths[0]).map_err(|e| format!("cannot read `{}`: {e}", paths[0]))?;
        if meta.is_dir() {
            let dir = &paths[0];
            let mut entries: Vec<String> = std::fs::read_dir(dir)
                .map_err(|e| format!("cannot read directory `{dir}`: {e}"))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map(|x| x == "swift").unwrap_or(false))
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            entries.sort();
            if entries.is_empty() {
                return Err(format!("no `.swift` files found in `{dir}`"));
            }
            return read_all(&entries);
        }
    }
    read_all(paths)
}

/// Read every path into a [`SourceFile`], preserving order.
fn read_all(paths: &[String]) -> Result<Vec<SourceFile>, String> {
    paths
        .iter()
        .map(|path| {
            std::fs::read_to_string(path)
                .map(|source| SourceFile::new(path.clone(), source))
                .map_err(|e| format!("cannot read `{path}`: {e}"))
        })
        .collect()
}

/// Analyze and evaluate the Swift program at `paths` (files or a directory).
/// Multiple files form one compilation unit analyzed via
/// [`Analysis::analyze_program`], so cross-file references resolve and
/// diagnostics report the correct `file:line`.
fn run(paths: &[String], allow_network: bool) -> ExitCode {
    let files = match collect_source_files(paths) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Entry file for runtime diagnostics: the `main.swift` if present, else the
    // first file in order.
    let path = files
        .iter()
        .find(|f| {
            std::path::Path::new(&f.path)
                .file_name()
                .map(|n| n == "main.swift")
                .unwrap_or(false)
        })
        .or_else(|| files.first())
        .map(|f| f.path.clone())
        .unwrap_or_else(|| "main.swift".to_string());
    let path = path.as_str();

    let analysis = match Analysis::analyze_program(&files) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Surface diagnostics: warnings (e.g. `#warning`) print and continue;
    // errors (e.g. `#error`, type errors) abort before execution. Each
    // diagnostic reports its own file path (multi-file aware).
    let mut had_error = false;
    for diag in analysis.diagnostics() {
        let kind = if diag.is_error() { "error" } else { "warning" };
        let file = diag.file.as_deref().unwrap_or(path);
        eprintln!(
            "{file}:{}:{}: {kind}: {}",
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
    // persistence with `TSWIFT_DEFAULTS_FILE`; see `defaults.rs`); `FileManager`
    // is backed by the real filesystem, unrooted (see `fs.rs`). Both route
    // through one combined handler (`host.rs`) since the host bridge supports a
    // single default handler; it must be installed before `install_with`
    // declares the host-fn signatures that route through it.
    interp.set_host_call_handler(std::sync::Arc::new(host::CliHostHandler::new()));
    let caps = tswift_core::Capabilities::all();
    tswift_foundation::install_with(&mut interp, caps);
    // `tswift-swiftdata` only declares the `tswift.db.*` wire signatures at
    // this slice (no Swift-facing `SwiftData` API yet); `db::DbHandler`
    // above (routed through `host::CliHostHandler`) backs them with real
    // SQLite.
    tswift_swiftdata::install(
        &mut interp,
        caps.contains(tswift_core::HostService::Database),
    );
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
    // Drop the interpreter before flushing/reusing the stdout handle: this runs
    // any registered finalizers (e.g. closing SwiftData database handles) and
    // releases the `&mut handle` borrow.
    drop(interp);
    let _ = handle.flush();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
