//! `quick-swift` — the command-line entry point.
//!
//! Usage:
//!   quick-swift run <file.swift> [more.swift ...]
//!   quick-swift dump [--json] <file.swift>
//!
//! `run` analyzes a Swift source file with msf and evaluates it through the
//! quick-swift runtime, streaming program output to stdout. `dump` prints the
//! typed AST (kind, text, line, resolved type, modifiers) for inspecting how msf
//! parses a construct — the fast path when adding a language feature.

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
        Some("dump") => {
            let rest: Vec<String> = args.collect();
            match parse_dump_args(&rest) {
                Ok(DumpArgs { path, json }) => dump(&path, json),
                Err(msg) => {
                    eprintln!("error: {msg}\n\nusage: quick-swift dump [--json] <file.swift>");
                    ExitCode::FAILURE
                }
            }
        }
        Some(other) => {
            eprintln!(
                "error: unknown command `{other}`\n\nusage: quick-swift run <file.swift> | quick-swift dump [--json] <file.swift>"
            );
            ExitCode::FAILURE
        }
        None => {
            eprintln!(
                "usage: quick-swift run <file.swift> [more.swift ...]\n       quick-swift dump [--json] <file.swift>"
            );
            ExitCode::FAILURE
        }
    }
}

/// Parsed `dump` subcommand arguments: exactly one input path, plus flags.
struct DumpArgs {
    path: String,
    json: bool,
}

/// Parse `dump` arguments strictly: accept the `--json` flag and exactly one
/// input path. Reject unknown flags and extra paths with a clear message rather
/// than silently ignoring them.
fn parse_dump_args(args: &[String]) -> Result<DumpArgs, String> {
    let mut json = false;
    let mut path: Option<String> = None;
    for arg in args {
        if let Some(flag) = arg.strip_prefix("--") {
            match flag {
                "json" => json = true,
                other => return Err(format!("unknown flag `--{other}`")),
            }
        } else if path.is_none() {
            path = Some(arg.clone());
        } else {
            return Err(format!(
                "`dump` accepts a single file path, but got an extra argument `{arg}`"
            ));
        }
    }
    match path {
        Some(path) => Ok(DumpArgs { path, json }),
        None => Err("`dump` requires a file path".to_string()),
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
    let analysis = match Analysis::analyze(&source, path) {
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
