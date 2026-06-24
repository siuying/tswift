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
        Some("run") => match args.next() {
            Some(path) => run(&path),
            None => {
                eprintln!(
                    "error: `run` requires a file path\n\nusage: quick-swift run <file.swift>"
                );
                ExitCode::FAILURE
            }
        },
        Some(other) => {
            eprintln!("error: unknown command `{other}`\n\nusage: quick-swift run <file.swift>");
            ExitCode::FAILURE
        }
        None => {
            eprintln!("usage: quick-swift run <file.swift>");
            ExitCode::FAILURE
        }
    }
}

/// Analyze and evaluate the Swift file at `path`.
fn run(path: &str) -> ExitCode {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read `{path}`: {e}");
            return ExitCode::FAILURE;
        }
    };

    let analysis = match Analysis::analyze(&source, path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let mut interp = Interpreter::new(&mut handle);
    quick_swift_std::install(&mut interp);

    let result = interp.run(&analysis);
    let _ = handle.flush();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
