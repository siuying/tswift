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

use std::collections::HashSet;
use std::process::ExitCode;
use std::time::Duration;

use tswift_frontend::SourceFile;
use tswift_testing::{CompileError, RunOptions, RunReport, TestDescriptor, TestStatus};

/// The parsed `tswift test` argument list: `--filter`/`--target` values plus
/// the positional path arguments, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestArgs {
    pub filter: Option<String>,
    pub target: Option<String>,
    pub paths: Vec<String>,
    /// `--list`: print the discovered tests instead of running them.
    pub list: bool,
    /// `--json`: with `--list`, emit machine-readable JSON.
    pub json: bool,
    /// `--test <id>` (repeatable): exact canonical ids to run.
    pub tests: Vec<String>,
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
            "--test" => {
                out.tests.push(
                    it.next()
                        .ok_or_else(|| "`--test` requires a value".to_string())?
                        .clone(),
                );
            }
            "--list" => out.list = true,
            "--json" => out.json = true,
            flag if flag.starts_with("--") => {
                return Err(format!("unknown flag `{flag}`"));
            }
            path => out.paths.push(path.to_string()),
        }
    }
    // `--test` (exact id selection) and `--filter` (substring match) are
    // mutually exclusive: pick one. Combining them is a usage error rather
    // than a silently-resolved precedence.
    if !out.tests.is_empty() && out.filter.is_some() {
        return Err("`--test` and `--filter` are mutually exclusive".to_string());
    }
    // `--list` only discovers tests; `--test`/`--filter` only make sense for
    // a run. Combining them is a usage error, not a silently-ignored
    // selection (which would read as if the flag were honored).
    if out.list && (!out.tests.is_empty() || out.filter.is_some()) {
        return Err("`--list` cannot be combined with `--test`/`--filter`".to_string());
    }
    Ok(out)
}

/// Run `tswift test` over the parsed arguments, printing console output and
/// returning the process exit code. In `--list` mode it prints the discovered
/// tests (human table or, with `--json`, machine-readable JSON) without
/// running any.
pub fn run(args: &TestArgs) -> ExitCode {
    let units = match collect_test_units(&args.paths, args.target.as_deref()) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if args.list {
        return list(&units, args.json);
    }

    let filter = args.filter.as_deref();

    // `--test <id>` selection is resolved *across* units before any unit
    // runs (only meaningful with more than one unit): each requested id must
    // match in at least one unit (a true unknown-everywhere id is still a
    // hard error), but a unit that doesn't happen to have a given id is not
    // an error for that unit — it just runs the subset of requested ids it
    // does have, or is skipped entirely if it has none of them. A single-unit
    // run keeps the simple original behavior (every requested id passed
    // straight through; an id unknown to that one unit is `run_tests`'s own
    // "unknown test id(s)" error).
    let per_unit_ids: Vec<Option<Vec<String>>> = if args.tests.is_empty() {
        vec![None; units.len()]
    } else if units.len() <= 1 {
        vec![Some(args.tests.clone())]
    } else {
        match resolve_ids_across_units(&units, &args.tests) {
            Ok(per_unit) => per_unit,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
    };

    let mut all_ok = true;
    let mut total_tests = 0usize;
    let mut total_failed = 0usize;
    let mut total_skipped = 0usize;
    let mut total_issues = 0usize;
    let mut total_duration = Duration::ZERO;

    for ((name, files), ids) in units.iter().zip(per_unit_ids) {
        // An id selection that resolved to "none of the requested ids belong
        // to this unit" skips the unit entirely — no header, no run, not
        // counted — rather than running it unfiltered or erroring on ids it
        // was never asked to have.
        if let Some(selected) = &ids {
            if selected.is_empty() {
                continue;
            }
        }
        if units.len() > 1 {
            println!("Test target {name}:");
        }
        let options = RunOptions {
            filter: args.filter.clone(),
            ids,
        };
        let report = tswift_testing::run_tests(files, &options);
        if let Some(err) = &report.compile_error {
            render_compile_error(err, files, name);
            all_ok = false;
            continue;
        }
        print!("{}", render_report(&report, filter));
        all_ok &= report.is_success();
        total_tests += report.tests.len();
        total_failed += report.failed();
        total_skipped += report.skipped();
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
            overall_summary(
                total_tests,
                total_failed,
                total_skipped,
                total_issues,
                total_duration,
                filter
            )
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

/// For each unit, the known selectable ids (a test's own id, plus every one
/// of its parameterized case ids), when the unit can be listed at all. A unit
/// that fails to list (compile error) yields `None`: its selection can't be
/// resolved, so [`resolve_ids_across_units`] always includes it unfiltered,
/// letting the unit's own compile error surface when it actually runs rather
/// than being silently skipped for "having none of the requested ids".
fn known_ids_per_unit(units: &[(String, Vec<SourceFile>)]) -> Vec<Option<HashSet<String>>> {
    units
        .iter()
        .map(|(_, files)| {
            tswift_testing::list_tests(files).ok().map(|descriptors| {
                let mut ids = HashSet::new();
                for d in &descriptors {
                    ids.insert(d.id.clone());
                    ids.extend(d.cases.iter().cloned());
                }
                ids
            })
        })
        .collect()
}

/// Resolve a `--test <id>` selection across every unit in a multi-unit run:
/// each requested id must match in at least one unit (an id matching *no*
/// unit is a hard error, naming the unknown id — never a silent zero-tests
/// success); a unit that doesn't have a given id is not an error for that
/// unit. Returns one `Option<Vec<String>>` per unit, aligned with `units`:
/// `None` for a unit that couldn't be listed (always run unfiltered, so its
/// own compile error surfaces); `Some(ids)` — possibly empty, meaning "skip
/// this unit" — for a unit that could.
fn resolve_ids_across_units(
    units: &[(String, Vec<SourceFile>)],
    requested: &[String],
) -> Result<Vec<Option<Vec<String>>>, String> {
    let known = known_ids_per_unit(units);

    // A requested id is truly unknown only if every listable unit lacks it;
    // a unit that couldn't be listed (`None`) means we can't rule the id out,
    // so its presence there is deferred to that unit's own run.
    let any_unlistable = known.iter().any(Option::is_none);
    if !any_unlistable {
        let unknown: Vec<&String> = requested
            .iter()
            .filter(|id| {
                !known
                    .iter()
                    .any(|set| set.as_ref().unwrap().contains(id.as_str()))
            })
            .collect();
        if !unknown.is_empty() {
            let names: Vec<&str> = unknown.iter().map(|s| s.as_str()).collect();
            return Err(format!("unknown test id(s): {}", names.join(", ")));
        }
    }

    Ok(known
        .iter()
        .map(|set| {
            set.as_ref().map(|set| {
                requested
                    .iter()
                    .filter(|id| set.contains(id.as_str()))
                    .cloned()
                    .collect()
            })
        })
        .collect())
}

/// Print the discovered tests across every unit without running them. With
/// `json`, emit one combined document (the same wire shape the wasm
/// `listTests` / FFI `tswift_list_tests` seams return; see
/// [`tswift_testing::list_units_to_json`]) — `ok:false` and a `compileError`
/// when any unit fails to list, never a silent empty/partial success; else a
/// human table, one test per line (plus one line per parameterized case's own
/// selectable id), grouped by unit when there is more than one. A compile
/// error is rendered the same way `run` renders one and makes the whole
/// command exit nonzero.
fn list(units: &[(String, Vec<SourceFile>)], json: bool) -> ExitCode {
    let mut named: Vec<(String, Result<Vec<TestDescriptor>, CompileError>)> = Vec::new();
    let mut any_error = false;
    for (name, files) in units {
        let mut result = tswift_testing::list_tests(files);
        if let Ok(tests) = &mut result {
            if units.len() > 1 {
                for t in tests.iter_mut() {
                    t.target = Some(name.clone());
                }
            }
        }
        match &result {
            Ok(tests) => {
                if !json {
                    if units.len() > 1 {
                        println!("Test target {name}:");
                    }
                    for t in tests {
                        println!("{}", list_line(t));
                        for case_id in &t.cases {
                            println!("  {case_id}");
                        }
                    }
                }
            }
            Err(err) => {
                any_error = true;
                render_compile_error(err, files, name);
            }
        }
        named.push((name.clone(), result));
    }
    if json {
        println!("{}", tswift_testing::list_units_to_json(&named));
    }
    if any_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// One human-readable line for a discovered test: id, a `[N cases]` badge for a
/// parameterized test, `#tag` badges, and a `(skipped: reason)` suffix for a
/// statically-disabled test.
fn list_line(t: &TestDescriptor) -> String {
    let mut line = t.id.clone();
    if let Some(name) = &t.display_name {
        line.push_str(&format!("  \"{name}\""));
    }
    if let Some(n) = t.case_count {
        line.push_str(&format!("  [{n} case{}]", if n == 1 { "" } else { "s" }));
    }
    for tag in &t.tags {
        line.push_str(&format!("  #{tag}"));
    }
    if t.skipped {
        match &t.skip_reason {
            Some(reason) => line.push_str(&format!("  (skipped: {reason})")),
            None => line.push_str("  (skipped)"),
        }
    }
    line
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
/// `filter` is the run's `--filter` value (if any), surfaced in the summary
/// when it excluded every test so a zero-test exit reads clearly instead of
/// silently — a `tag:<name>` filter that matches nothing is otherwise
/// indistinguishable from a directory with no `@Test`s at all.
fn render_report(report: &RunReport, filter: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("Test run started.\n");
    for test in &report.tests {
        let secs = test.duration.as_secs_f64();
        // Known issues (from `withKnownIssue`) are reported for both passing
        // and failing tests, but never fail the run — mark them distinctly.
        for issue in test.issues.iter().filter(|i| i.known) {
            out.push_str(&format!(
                "\u{25c7} Test {} recorded a known issue at {}: {}\n",
                test.label(),
                issue_loc(issue),
                issue.message
            ));
        }
        match test.status {
            TestStatus::Passed => {
                out.push_str(&format!(
                    "\u{2714} Test {} passed after {secs:.3} seconds.\n",
                    test.label()
                ));
            }
            TestStatus::Skipped => match &test.skip_reason {
                Some(reason) => out.push_str(&format!(
                    "\u{21b7} Test {} skipped: \"{reason}\".\n",
                    test.label()
                )),
                None => out.push_str(&format!("\u{21b7} Test {} skipped.\n", test.label())),
            },
            TestStatus::Failed => {
                let failing: Vec<_> = test.issues.iter().filter(|i| !i.known).collect();
                for issue in &failing {
                    out.push_str(&format!(
                        "\u{2718} Test {} recorded an issue at {}: {}\n",
                        test.label(),
                        issue_loc(issue),
                        issue.message
                    ));
                }
                // Surface any `.bug(…)` references on a failing test (plan §1.2).
                for bug in &test.bugs {
                    out.push_str(&format!("  bug: {bug}\n"));
                }
                out.push_str(&format!(
                    "\u{2718} Test {} failed after {secs:.3} seconds with {} issue{}.\n",
                    test.label(),
                    failing.len(),
                    plural(failing.len())
                ));
            }
        }
    }
    out.push_str(&overall_summary(
        report.tests.len(),
        report.failed(),
        report.skipped(),
        report.issue_count(),
        report.duration,
        filter,
    ));
    out.push('\n');
    out
}

/// A `file:line` location for an issue, or `<unknown>:line` when the file is
/// not known.
fn issue_loc(issue: &tswift_testing::Issue) -> String {
    match &issue.file {
        Some(file) => format!("{file}:{}", issue.line),
        None => format!("<unknown>:{}", issue.line),
    }
}

/// The final `Test run with N tests …` summary line (also used to combine
/// totals across multiple test-target units).
fn overall_summary(
    tests: usize,
    failed: usize,
    skipped: usize,
    issues: usize,
    duration: Duration,
    filter: Option<&str>,
) -> String {
    let secs = duration.as_secs_f64();
    let skips = if skipped == 0 {
        String::new()
    } else {
        format!(" ({skipped} skipped)")
    };
    if tests == 0 {
        // Name the filter that excluded everything (e.g. a `tag:<name>` with
        // no matching test) rather than a bare "nothing matched", which reads
        // identically to a directory with no `@Test`s at all.
        let cause = match filter {
            Some(f) => format!("nothing matched --filter {f}"),
            None => "nothing matched".to_string(),
        };
        format!("Test run with 0 tests ({cause}) after {secs:.3} seconds.")
    } else if failed == 0 {
        // `tests` includes any skipped tests; "N tests, M passed" must count
        // only the ones that actually ran and passed, with skips called out
        // separately (`skips`), not folded into the passed count.
        let passed = tests - skipped;
        format!(
            "Test run with {tests} test{}, {passed} passed after {secs:.3} seconds{skips}.",
            plural(tests)
        )
    } else {
        format!(
            "Test run with {tests} test{} failed after {secs:.3} seconds with {issues} issue{}{skips}.",
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
            skip_reason: None,
            bugs: Vec::new(),
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
                known: false,
            }],
            duration: Duration::from_millis(1),
            file: Some("t.swift".to_string()),
            line: 1,
            skip_reason: None,
            bugs: Vec::new(),
        }
    }

    #[test]
    fn render_passing_report_shows_pass_marker_and_summary() {
        let report = RunReport {
            tests: vec![passing("a()")],
            duration: Duration::from_millis(2),
            compile_error: None,
        };
        let out = render_report(&report, None);
        assert!(out.contains("Test a() passed"), "{out}");
        assert!(out.contains("Test run with 1 test, 1 passed"), "{out}");
    }

    #[test]
    fn render_failing_report_shows_issue_location_and_count() {
        let report = RunReport {
            tests: vec![failing("b()")],
            duration: Duration::from_millis(2),
            compile_error: None,
        };
        let out = render_report(&report, None);
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
    fn skipped_tests_are_not_counted_as_passed_in_summary() {
        // A run with a pass and a skip (no failures) must not word its summary
        // as "N tests passed" when only some of them actually ran and passed.
        let mut skipped = passing("b()");
        skipped.status = TestStatus::Skipped;
        skipped.skip_reason = Some("flaky".to_string());
        let report = RunReport {
            tests: vec![passing("a()"), skipped],
            duration: Duration::from_millis(2),
            compile_error: None,
        };
        let out = render_report(&report, None);
        assert!(out.contains("Test run with 2 tests, 1 passed"), "{out}");
    }

    #[test]
    fn zero_tests_summary_reads_explicitly() {
        let report = RunReport {
            tests: Vec::new(),
            duration: Duration::ZERO,
            compile_error: None,
        };
        let out = render_report(&report, None);
        assert!(out.contains("0 tests"), "{out}");
        assert!(report.is_success(), "zero tests is a success (plan §2.5)");
    }

    #[test]
    fn zero_tests_summary_names_the_tag_filter() {
        // A `--filter tag:<name>` that matches nothing must not exit
        // silently: the summary names the filter, not just "nothing
        // matched", so a tag typo is diagnosable from the console output.
        let report = RunReport {
            tests: Vec::new(),
            duration: Duration::ZERO,
            compile_error: None,
        };
        let out = render_report(&report, Some("tag:nope"));
        assert!(
            out.contains("0 tests (nothing matched --filter tag:nope)"),
            "{out}"
        );
    }
}
