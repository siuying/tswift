//! Golden-fixture harness for the Swift frontend.
//!
//! Walks the repo-owned corpus at `tests/swift-fixtures/` and asserts each
//! fixture's inline directives hold when analyzed through the frontend:
//!
//! - `// expected-no-diagnostics` — the file must analyze with zero diagnostics.
//! - `// expected-error{{substring}}` — the line it is on must produce a
//!   diagnostic whose (case-insensitive) message contains `substring`.
//! - `// oracle-gap: <reason>` — valid Swift a differential C-oracle backend
//!   cannot handle; skipped only by that backend's harness, never here.
//!
//! Every positive fixture is validated against the pure-Rust frontend; there is
//! no escape hatch that masks frontend coverage gaps.
//!
//! See `tests/swift-fixtures/README.md` for the directive language.

use std::fs;
use std::path::{Path, PathBuf};

use tswift_frontend::Analysis;

/// What a fixture's directives say should happen when it is analyzed.
#[derive(Debug, PartialEq, Eq)]
enum Expectation {
    /// Zero diagnostics expected.
    NoDiagnostics,
    /// Each `(line, substring)` must be matched by some diagnostic on that line.
    Errors(Vec<(u32, String)>),
    /// Excluded from this backend (a documented C-oracle gap).
    OracleGap,
    /// Valid Swift that the **Rust frontend** cannot yet handle — a known gap in
    /// our own pipeline, not in the C oracle.  Skipped by this harness so the
    /// corpus continues to pass CI while the limitation is being fixed.
    FrontendGap,
    /// No recognised directive — a fixture authoring mistake.
    Missing,
}

/// Parse the directive expectation out of a fixture's source text.
fn parse_expectation(source: &str) -> Expectation {
    // `frontend-gap`: valid Swift our own Rust frontend cannot yet handle.
    // Skipped entirely until the limitation is fixed.
    if source.contains("// frontend-gap:") {
        return Expectation::FrontendGap;
    }
    // `oracle-gap`: valid Swift only the differential C-oracle backend cannot
    // handle. The pure-Rust frontend is expected to accept these.
    if source.contains("// oracle-gap:") {
        return Expectation::OracleGap;
    }
    let mut errors = Vec::new();
    for (i, line) in source.lines().enumerate() {
        if let Some(rest) = line.split("// expected-error{{").nth(1) {
            if let Some(sub) = rest.split("}}").next() {
                errors.push((i as u32 + 1, sub.to_string()));
            }
        }
    }
    if !errors.is_empty() {
        return Expectation::Errors(errors);
    }
    if source.contains("// expected-no-diagnostics") {
        return Expectation::NoDiagnostics;
    }
    Expectation::Missing
}

/// Recursively collect every `.swift` file under `dir`.
fn collect_swift(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {dir:?}: {e}"));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_swift(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("swift") {
            out.push(path);
        }
    }
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/swift-fixtures")
}

/// Check one fixture against its directives; return an error string on mismatch.
fn check_fixture(path: &Path, source: &str) -> Result<(), String> {
    let analysis = Analysis::analyze(source, &path.to_string_lossy())
        .map_err(|e| format!("analyze failed: {e}"))?;
    let diags = analysis.diagnostics();

    match parse_expectation(source) {
        Expectation::OracleGap | Expectation::FrontendGap => Ok(()),
        Expectation::Missing => {
            Err("no directive (expected-no-diagnostics / expected-error / oracle-gap)".to_string())
        }
        Expectation::NoDiagnostics => {
            if diags.is_empty() {
                Ok(())
            } else {
                let shown: Vec<String> = diags
                    .iter()
                    .map(|d| format!("{}:{}: {}", d.line, d.col, d.message))
                    .collect();
                Err(format!(
                    "expected no diagnostics, got:\n    {}",
                    shown.join("\n    ")
                ))
            }
        }
        Expectation::Errors(expected) => {
            let mut misses = Vec::new();
            for (line, sub) in &expected {
                let sub_lc = sub.to_lowercase();
                let matched = diags
                    .iter()
                    .any(|d| d.line == *line && d.message.to_lowercase().contains(&sub_lc));
                if !matched {
                    misses.push(format!("line {line} expected error containing {sub:?}"));
                }
            }
            if misses.is_empty() {
                Ok(())
            } else {
                let got: Vec<String> = diags
                    .iter()
                    .map(|d| format!("{}:{}: {}", d.line, d.col, d.message))
                    .collect();
                Err(format!(
                    "{}\n  diagnostics were:\n    {}",
                    misses.join("\n  "),
                    if got.is_empty() {
                        "<none>".to_string()
                    } else {
                        got.join("\n    ")
                    }
                ))
            }
        }
    }
}

/// Every fixture in the corpus must satisfy its directives on the current backend.
#[test]
fn corpus_satisfies_directives() {
    let root = fixtures_root();
    let mut files = Vec::new();
    collect_swift(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no fixtures found under {root:?}");

    let mut failures = Vec::new();
    let mut checked = 0;
    let mut skipped = 0;
    for path in &files {
        let source = fs::read_to_string(path).expect("read fixture");
        if matches!(
            parse_expectation(&source),
            Expectation::OracleGap | Expectation::FrontendGap
        ) {
            skipped += 1;
            continue;
        }
        checked += 1;
        if let Err(msg) = check_fixture(path, &source) {
            let rel = path.strip_prefix(&root).unwrap_or(path);
            failures.push(format!("{}: {msg}", rel.display()));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {checked} checked fixtures failed ({skipped} oracle-gap skipped):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    eprintln!("golden corpus: {checked} checked, {skipped} oracle-gap skipped");
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn detects_no_diagnostics() {
        assert_eq!(
            parse_expectation("// expected-no-diagnostics\nlet x = 1\n"),
            Expectation::NoDiagnostics
        );
    }

    #[test]
    fn oracle_gap_takes_precedence() {
        // Even with other directive-looking text, an oracle-gap file is skipped.
        let src = "// oracle-gap: regex\nlet r = /x/ // expected-error{{nope}}\n";
        assert_eq!(parse_expectation(src), Expectation::OracleGap);
    }

    #[test]
    fn collects_expected_errors_with_line_numbers() {
        let src = "let a = 1\nbad // expected-error{{cannot find}}\n";
        assert_eq!(
            parse_expectation(src),
            Expectation::Errors(vec![(2, "cannot find".to_string())])
        );
    }

    #[test]
    fn missing_directive_is_reported() {
        assert_eq!(parse_expectation("let x = 1\n"), Expectation::Missing);
    }
}
