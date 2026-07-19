//! `tswift test` — the `tswift-testing` runner's CLI entry (Slice B,
//! `docs/plan/swift-testing-support.md` §4).
//!
//! Reuses the same path-expansion rules as `run` (files / sorted flat
//! directory / `Package.swift`), but a `Package.swift` project selects
//! `.testTarget`s (via [`tswift_frontend::project::load_test_program`])
//! instead of the sole `.executableTarget`: every declared test target runs
//! sequentially by default, or `--target <name>` selects one. Console
//! rendering mirrors `swift test`'s shape (suite/test lines, issue detail at
//! `file:line`, a final summary) with ASCII pass/fail/skip markers instead of
//! SF Symbol codepoints, so output stays readable in any CI log.
//!
//! **Zero-discovered-tests policy** (plan §2.5 / R3): a run that discovers no
//! tests is *not* an error — it exits `0` with an explicit "0 tests" summary
//! line, so an empty or over-filtered run reads clearly in a log without
//! silently going CI-green on a *broken* run (which is still caught: a
//! compile error or any failing test is always nonzero).

use std::process::ExitCode;
use std::time::Duration;

use tswift_frontend::SourceFile;
use tswift_testing::{CompileError, RunOptions, RunReport, TestStatus};

/// The parsed `tswift test` argument list: `--filter`/`--target` values plus
/// the positional path arguments, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestArgs {
    pub filter: Option<String>,
    pub target: Option<String>,
    pub paths: Vec<String>,
}

/// Parse `tswift test`'s argument list.
///
/// `--filter <substring>` and `--target <name>` each consume the following
/// argument as their value; every other `--`-prefixed argument is an unknown
/// flag (`Err`), never silently dropped. `--filter`/`--target` as the final
/// argument (no following value) is also `Err`, never a silent `None`.
/// Anything else is a positional path argument.
pub fn parse_test_args(rest: &[String]) -> Result<TestArgs, String> {
    let mut out = TestArgs::default();
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--filter" => {
                out.filter = Some(
                    it.next()
                        .ok_or_else(|| "`--filter` requires a value".to_string())?
                        .clone(),
                );
            }
            "--target" => {
                out.target = Some(
                    it.next()
                        .ok_or_else(|| "`--target` requires a value".to_string())?
                        .clone(),
                );
            }
            flag if flag.starts_with("--") => {
                return Err(format!("unknown flag `{flag}`"));
            }
            path => out.paths.push(path.to_string()),
        }
    }
    Ok(out)
}

/// Run `tswift test` over `paths`, printing console output and returning the
/// process exit code.
pub fn run(paths: &[String], filter: Option<&str>, target: Option<&str>) -> ExitCode {
    let units = match collect_test_units(paths, target) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let options = RunOptions {
        filter: filter.map(str::to_string),
    };
    let mut all_ok = true;
    let mut total_tests = 0usize;
    let mut total_failed = 0usize;
    let mut total_issues = 0usize;
    let mut total_duration = Duration::ZERO;

    for (name, files) in &units {
        if units.len() > 1 {
            println!("Test target {name}:");
        }
        let report = tswift_testing::run_tests(files, &options);
        if let Some(err) = &report.compile_error {
            render_compile_error(err, files, name);
            all_ok = false;
            continue;
        }
        print!("{}", render_report(&report));
        all_ok &= report.is_success();
        total_tests += report.tests.len();
        total_failed += report.failed();
        total_issues += report.issue_count();
        total_duration += report.duration;
    }

    if units.len() > 1 {
        // Each unit already printed its own "Test run with N tests …" summary
        // (`render_report`); label the cross-unit total distinctly ("Overall:")
        // so it reads as a combined total, not a third, contradictory-looking
        // "Test run with…" line.
        println!(
            "Overall: {}",
            overall_summary(total_tests, total_failed, total_issues, total_duration)
                .strip_prefix("Test run with ")
                .expect("overall_summary always starts with \"Test run with \"")
        );
    }

    if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Render a [`CompileError`] the same way `tswift run` renders analysis
/// diagnostics (`render_diagnostic`: `file:line:col: error: msg` + source
/// line + caret), so a compile failure reads identically whether it came from
/// `tswift run` or `tswift test`. `unit_name` is the fallback path for a
/// diagnostic that (defensively) carries no `file` of its own; `files` is the
/// unit's own source set, used to look up the offending line for the caret.
fn render_compile_error(err: &CompileError, files: &[SourceFile], unit_name: &str) {
    match err {
        CompileError::Diagnostics(diags) => {
            let fallback = files.first().map(|f| f.path.as_str()).unwrap_or(unit_name);
            for diag in diags {
                eprint!("{}", crate::render_diagnostic(diag, files, fallback));
            }
        }
        CompileError::Message(msg) => eprintln!("{msg}"),
    }
}

/// Expand `paths` into one or more named test units: `(target_name, files)`,
/// `target_name` empty for a plain file/directory input (no target concept).
/// A directory containing `Package.swift` loads every selected `.testTarget`
/// (see module docs); anything else is a single unnamed unit, same expansion
/// as `tswift run`.
fn collect_test_units(
    paths: &[String],
    target: Option<&str>,
) -> Result<Vec<(String, Vec<SourceFile>)>, String> {
    if paths.len() == 1 {
        let meta =
            std::fs::metadata(&paths[0]).map_err(|e| format!("cannot read `{}`: {e}", paths[0]))?;
        if meta.is_dir() {
            let root = std::path::Path::new(&paths[0]);
            if root.join("Package.swift").is_file() {
                let mut entries = Vec::new();
                crate::collect_project_files_recursive(root, root, &mut entries)?;
                let programs = tswift_frontend::project::load_test_program(&entries, target)
                    .map_err(|e| e.to_string())?;
                return Ok(programs.into_iter().map(|p| (p.target, p.files)).collect());
            }
        }
    }
    let files = crate::collect_source_files(paths, target)?;
    Ok(vec![(String::new(), files)])
}

/// Render one unit's [`RunReport`] as `swift test`-shaped console output:
/// a `Test run started.` line, one line per test (plus an issue line per
/// recorded failure, each carrying `file:line`), and a final summary line.
fn render_report(report: &RunReport) -> String {
    let mut out = String::new();
    out.push_str("Test run started.\n");
    for test in &report.tests {
        let secs = test.duration.as_secs_f64();
        match test.status {
            TestStatus::Passed => {
                out.push_str(&format!(
                    "\u{2714} Test {} passed after {secs:.3} seconds.\n",
                    test.label()
                ));
            }
            TestStatus::Skipped => {
                out.push_str(&format!("\u{21b7} Test {} skipped.\n", test.label()));
            }
            TestStatus::Failed => {
                for issue in &test.issues {
                    let loc = match &issue.file {
                        Some(file) => format!("{file}:{}", issue.line),
                        None => format!("<unknown>:{}", issue.line),
                    };
                    out.push_str(&format!(
                        "\u{2718} Test {} recorded an issue at {loc}: {}\n",
                        test.label(),
                        issue.message
                    ));
                }
                out.push_str(&format!(
                    "\u{2718} Test {} failed after {secs:.3} seconds with {} issue{}.\n",
                    test.label(),
                    test.issues.len(),
                    plural(test.issues.len())
                ));
            }
        }
    }
    out.push_str(&overall_summary(
        report.tests.len(),
        report.failed(),
        report.issue_count(),
        report.duration,
    ));
    out.push('\n');
    out
}

/// The final `Test run with N tests …` summary line (also used to combine
/// totals across multiple test-target units).
fn overall_summary(tests: usize, failed: usize, issues: usize, duration: Duration) -> String {
    let secs = duration.as_secs_f64();
    if tests == 0 {
        format!("Test run with 0 tests (nothing matched) after {secs:.3} seconds.")
    } else if failed == 0 {
        format!(
            "Test run with {tests} test{} passed after {secs:.3} seconds.",
            plural(tests)
        )
    } else {
        format!(
            "Test run with {tests} test{} failed after {secs:.3} seconds with {issues} issue{}.",
            plural(tests),
            plural(issues)
        )
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_testing::{Issue, TestResult};

    fn passing(id: &str) -> TestResult {
        TestResult {
            id: id.to_string(),
            display_name: None,
            status: TestStatus::Passed,
            issues: Vec::new(),
            duration: Duration::from_millis(1),
            file: Some("t.swift".to_string()),
            line: 1,
        }
    }

    fn failing(id: &str) -> TestResult {
        TestResult {
            id: id.to_string(),
            display_name: None,
            status: TestStatus::Failed,
            issues: vec![Issue {
                message: "Expectation failed: 1 == 2 \u{2192} false".to_string(),
                file: Some("t.swift".to_string()),
                line: 3,
            }],
            duration: Duration::from_millis(1),
            file: Some("t.swift".to_string()),
            line: 1,
        }
    }

    #[test]
    fn render_passing_report_shows_pass_marker_and_summary() {
        let report = RunReport {
            tests: vec![passing("a()")],
            duration: Duration::from_millis(2),
            compile_error: None,
        };
        let out = render_report(&report);
        assert!(out.contains("Test a() passed"), "{out}");
        assert!(out.contains("Test run with 1 test passed"), "{out}");
    }

    #[test]
    fn render_failing_report_shows_issue_location_and_count() {
        let report = RunReport {
            tests: vec![failing("b()")],
            duration: Duration::from_millis(2),
            compile_error: None,
        };
        let out = render_report(&report);
        assert!(out.contains("t.swift:3"), "{out}");
        assert!(out.contains("1 == 2"), "{out}");
        assert!(out.contains("with 1 issue"), "{out}");
    }

    #[test]
    fn parse_args_reads_filter_target_and_paths() {
        let rest = vec![
            "--filter".to_string(),
            "alpha".to_string(),
            "--target".to_string(),
            "CoreTests".to_string(),
            "dir".to_string(),
        ];
        let parsed = parse_test_args(&rest).unwrap();
        assert_eq!(parsed.filter.as_deref(), Some("alpha"));
        assert_eq!(parsed.target.as_deref(), Some("CoreTests"));
        assert_eq!(parsed.paths, vec!["dir".to_string()]);
    }

    #[test]
    fn parse_args_rejects_unknown_flag() {
        let rest = vec!["--bogus".to_string(), "dir".to_string()];
        let err = parse_test_args(&rest).unwrap_err();
        assert!(err.contains("--bogus"), "{err}");
    }

    #[test]
    fn parse_args_rejects_filter_with_no_value() {
        let rest = vec!["dir".to_string(), "--filter".to_string()];
        let err = parse_test_args(&rest).unwrap_err();
        assert!(err.contains("--filter"), "{err}");
    }

    #[test]
    fn parse_args_rejects_target_with_no_value() {
        let rest = vec!["dir".to_string(), "--target".to_string()];
        let err = parse_test_args(&rest).unwrap_err();
        assert!(err.contains("--target"), "{err}");
    }

    #[test]
    fn zero_tests_summary_reads_explicitly() {
        let report = RunReport {
            tests: Vec::new(),
            duration: Duration::ZERO,
            compile_error: None,
        };
        let out = render_report(&report);
        assert!(out.contains("0 tests"), "{out}");
        assert!(report.is_success(), "zero tests is a success (plan §2.5)");
    }
}
