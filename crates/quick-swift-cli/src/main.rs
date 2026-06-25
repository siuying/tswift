//! `quick-swift` — the command-line entry point.
//!
//! Usage:
//!   quick-swift run <file.swift>
//!
//! Reads a Swift source file, analyzes it with msf, and evaluates it through the
//! quick-swift runtime, streaming program output to stdout.

use std::io::{self, Write};
use std::process::ExitCode;

use msf::Analysis;
use quick_swift_core::Interpreter;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();

    match command.as_deref() {
        Some("run") => {
            let paths: Vec<String> = args.collect();
            if paths.is_empty() {
                eprintln!(
                    "error: `run` requires a file path\n\nusage: quick-swift run <file.swift> [more.swift ...]"
                );
                ExitCode::FAILURE
            } else {
                run(&paths)
            }
        }
        Some(other) => {
            eprintln!("error: unknown command `{other}`\n\nusage: quick-swift run <file.swift>");
            ExitCode::FAILURE
        }
        None => {
            eprintln!("usage: quick-swift run <file.swift> [more.swift ...]");
            ExitCode::FAILURE
        }
    }
}

/// Analyze and evaluate the Swift file(s) at `paths`. Multiple files form one
/// module: their sources are concatenated so cross-file references resolve.
fn run(paths: &[String]) -> ExitCode {
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

    let analysis = match Analysis::analyze(&source, path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    // The interpreter borrows the AST for `'static` (string interpolation leaks
    // small fragment ASTs to match). The process runs one program and exits, so
    // leaking the root analysis here is intentional and bounded.
    let analysis: &'static Analysis = Box::leak(Box::new(analysis));

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let mut interp = Interpreter::new(&mut handle);
    quick_swift_std::install(&mut interp);
    interp.set_filename(path);

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
