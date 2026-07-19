//! tswift-testing — a lightweight Swift Testing runner.
//!
//! The user-facing Swift Testing surface is *attributes + two freestanding
//! macros*: `@Test`/`@Suite` are discovered structurally from the typed AST,
//! and `#expect`/`#require` are interpreter macro builtins (same seam as
//! `#Predicate`). This crate needs no macro-expansion engine — discovery, a
//! serial runner, and two macro handlers (plan `docs/plan/swift-testing-support.md`).
//!
//! - [`install`] registers the `Testing` module surface on an interpreter.
//! - [`run_tests`] is the convenience entry the future `tswift test` CLI calls:
//!   it analyzes a program, installs the standard stack, discovers tests, and
//!   runs each in a fresh suite instance, returning a [`RunReport`].

mod discover;
mod expect;
mod params;
mod render;
mod report;
mod session;
mod traits;

use std::rc::Rc;
use std::time::Instant;

use tswift_core::{Interpreter, StdContext, StdError, SwiftValue};
use tswift_frontend::{Analysis, SourceFile};

use traits::Trait;

pub use discover::TestCase;
pub use report::{CompileError, Issue, RunReport, TestResult, TestStatus};

/// Options controlling a test run.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Case-sensitive substring filter on a test's id / display name; `None`
    /// runs every test.
    pub filter: Option<String>,
}

/// Register the `Testing` module surface (`#expect`, `#require`) on `interp`.
///
/// Mirrors `tswift_swiftdata::install`: the macros are scoped to the `Testing`
/// module so strict import-gating resolves them only after `import Testing`.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.module("Testing", |interp| {
        interp.register_macro("expect", expect::expect_macro);
        interp.register_macro("require", expect::require_macro);
        let issue = tswift_core::BuiltinReceiver::register_extension("Issue");
        interp.register_static(issue, "record", expect::issue_record);
    });
}

/// Analyze, load, discover, and run every `@Test` in `files`.
///
/// Builds a self-contained interpreter (standard library + Foundation +
/// Testing), auto-imports `Testing` so fixtures need not spell the import, and
/// runs each test serially in a fresh suite instance. Returns a structured
/// [`RunReport`]; a compile error yields a report with `compile_error` set and
/// no tests.
pub fn run_tests(files: &[SourceFile], options: &RunOptions) -> RunReport {
    let analysis = match Analysis::analyze_program(files) {
        Ok(analysis) => analysis,
        Err(err) => {
            return RunReport {
                compile_error: Some(CompileError::Message(err.to_string())),
                ..RunReport::default()
            }
        }
    };
    if !analysis.is_ok() {
        let diags: Vec<_> = analysis
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .cloned()
            .collect();
        return RunReport {
            compile_error: Some(CompileError::Diagnostics(diags)),
            ..RunReport::default()
        };
    }

    let mut sink = std::io::sink();
    let mut interp = Interpreter::new(&mut sink);
    tswift_std::install(&mut interp);
    tswift_foundation::install(&mut interp);
    install(&mut interp);
    // Auto-import Testing for the runner (plan §3.1, R5): fixtures may `import
    // Testing` but need not, since the runner always installs it.
    interp.mark_module_imported("Testing");

    let analysis: &'static Analysis = interp.retain_analysis(Rc::new(analysis));
    if let Err(err) = interp.load(analysis) {
        return RunReport {
            compile_error: Some(CompileError::Message(err.to_string())),
            ..RunReport::default()
        };
    }

    let mut cases = discover::discover(analysis.root());
    if let Some(filter) = &options.filter {
        cases.retain(|c| c.matches_filter(filter));
    }

    // Resolve each test into a flat list of plans — a skip, or one run per
    // parameterized case (a single run for an ordinary test). Only runs need a
    // synthetic driver call, so `.enabled(if:)` conditions are evaluated here
    // (against the loaded program) before any driver is built.
    let plans: Vec<Plan> = cases
        .iter()
        .flat_map(|c| plan_case(&mut interp, c))
        .collect();
    let driver_lines: Vec<String> = plans
        .iter()
        .filter_map(|p| match p {
            Plan::Run { driver, .. } => Some(driver.clone()),
            Plan::Skip { .. } | Plan::Fail { .. } => None,
        })
        .collect();

    // Build one synthetic driver holding a call per run, retained (not leaked)
    // for the interpreter's lifetime. A driver that fails to build must fail
    // the whole run — never silently pass tests.
    let driver_nodes = match build_drivers(&mut interp, &driver_lines) {
        Ok(nodes) => nodes,
        Err(err) => {
            return RunReport {
                compile_error: Some(CompileError::Message(err)),
                ..RunReport::default()
            }
        }
    };

    let run_start = Instant::now();
    let mut results = Vec::with_capacity(plans.len());
    let mut driver_nodes = driver_nodes.into_iter();
    for plan in &plans {
        match plan {
            Plan::Skip { case, reason } => {
                results.push(skipped_result(analysis, case, reason.clone()))
            }
            Plan::Fail { case, message } => {
                results.push(failed_result(analysis, case, message.clone()))
            }
            Plan::Run {
                case, id, label, ..
            } => {
                let node = driver_nodes.next().expect("one driver node per run");
                results.push(run_one(
                    &mut interp,
                    analysis,
                    case,
                    id,
                    label.clone(),
                    node,
                ));
            }
        }
    }
    RunReport {
        tests: results,
        duration: run_start.elapsed(),
        compile_error: None,
    }
}

/// A single unit of work for the runner: a trait-driven skip, a hard failure
/// discovered before any driver runs (a broken `.enabled(if:)` predicate),
/// or a concrete run (an ordinary test, or one parameterized case) carrying
/// its display id/label and the driver source line that invokes it.
enum Plan<'a> {
    Skip {
        case: &'a TestCase,
        reason: Option<String>,
    },
    Fail {
        case: &'a TestCase,
        message: String,
    },
    Run {
        case: &'a TestCase,
        id: String,
        label: Option<String>,
        driver: String,
    },
}

/// Turn one discovered `case` into its plans: a single skip, or one run per
/// parameterized argument case (a single run for an ordinary test).
fn plan_case<'a>(interp: &mut Interpreter<'_>, case: &'a TestCase) -> Vec<Plan<'a>> {
    match skip_reason(interp, case) {
        SkipDecision::Skip(reason) => return vec![Plan::Skip { case, reason }],
        SkipDecision::Fail(message) => return vec![Plan::Fail { case, message }],
        SkipDecision::Run => {}
    }
    match params::expand(&case.node) {
        params::Expansion::None => vec![Plan::Run {
            case,
            id: case.id(),
            label: Some(case.label_base()),
            driver: driver_line(case, ""),
        }],
        params::Expansion::Unsupported(spelling) => vec![Plan::Fail {
            case,
            message: format!(
                "unsupported arguments: {spelling} (expected collection literal or zip)"
            ),
        }],
        params::Expansion::Cases(rows) if rows.is_empty() => vec![Plan::Skip {
            case,
            reason: Some("no argument cases".to_string()),
        }],
        params::Expansion::Cases(rows) => rows
            .iter()
            .map(|row| {
                let args: Vec<String> = row.iter().map(|n| render::expr(n)).collect();
                let args = args.join(", ");
                let name = case
                    .display_name
                    .clone()
                    .unwrap_or_else(|| params::signature(&case.node, &case.func_name));
                let base = match &case.suite_display {
                    Some(suite) => format!("{suite}/{name}"),
                    None => name,
                };
                Plan::Run {
                    case,
                    id: format!("{} - {args}", case.id()),
                    label: Some(format!("{base} - {args}")),
                    driver: driver_line(case, &args),
                }
            })
            .collect(),
    }
}

/// The `try await`-wrapped driver statement that invokes `case` with `args`
/// (a comma-separated argument source, empty for a no-argument test). A suite
/// test constructs a fresh instance first (`Suite().method(args)`).
fn driver_line(case: &TestCase, args: &str) -> String {
    match &case.suite_type {
        Some(suite) => format!("try await {suite}().{}({args})", case.func_name),
        None => format!("try await {}({args})", case.func_name),
    }
}

/// The result of consulting `case`'s traits before building a driver.
enum SkipDecision {
    /// No skip-causing trait triggered; build a driver and run the test.
    Run,
    /// `.disabled("…")`, or a `.enabled(if:)` condition that evaluated to
    /// `false` (reason `None`) — not a failure.
    Skip(Option<String>),
    /// A `.enabled(if:)` condition that trapped, threw, or evaluated to a
    /// non-`Bool` value. A broken predicate must fail the test with a clear
    /// issue, never silently skip it (a skip stays CI-green).
    Fail(String),
}

/// Decide whether `case` is skipped/failed by a trait, or should run. The
/// first skip/fail-causing trait in source order wins.
fn skip_reason(interp: &mut Interpreter<'_>, case: &TestCase) -> SkipDecision {
    for trait_ in &case.traits {
        match trait_ {
            Trait::Disabled(reason) => return SkipDecision::Skip(reason.clone()),
            Trait::EnabledIf(cond) => {
                let outcome = {
                    let ctx: &mut dyn StdContext = interp;
                    ctx.eval_node(cond)
                };
                match outcome {
                    Ok(SwiftValue::Bool(true)) => {}
                    Ok(SwiftValue::Bool(false)) => return SkipDecision::Skip(None),
                    Ok(other) => {
                        return SkipDecision::Fail(format!(
                            "failed to evaluate .enabled(if:) condition: expected Bool, got {}",
                            other.type_name()
                        ))
                    }
                    Err(err) => {
                        return SkipDecision::Fail(format!(
                            "failed to evaluate .enabled(if:) condition: {}",
                            describe_error(&err)
                        ))
                    }
                }
            }
        }
    }
    SkipDecision::Run
}

/// A human-readable rendering of a `StdError` for use in a runner-authored
/// issue message (mirrors `run_one`'s outcome-to-issue mapping).
fn describe_error(err: &StdError) -> String {
    match err {
        StdError::Throw(value) => format!("condition threw an error: {value}"),
        StdError::Error(e) => e.to_string(),
    }
}

/// Build the [`TestResult`] for a test skipped by a trait (never a failure).
fn skipped_result(
    analysis: &'static Analysis,
    case: &TestCase,
    reason: Option<String>,
) -> TestResult {
    let (file, line) = analysis.locate(case.line);
    TestResult {
        id: case.id(),
        display_name: Some(case.label_base()),
        status: TestStatus::Skipped,
        issues: Vec::new(),
        duration: std::time::Duration::ZERO,
        file,
        line,
        skip_reason: reason,
    }
}

/// Build the [`TestResult`] for a test that failed before any driver ran (a
/// broken `.enabled(if:)` predicate) — carries a single synthetic [`Issue`]
/// with `message`, never a silent skip.
fn failed_result(analysis: &'static Analysis, case: &TestCase, message: String) -> TestResult {
    let (file, line) = analysis.locate(case.line);
    TestResult {
        id: case.id(),
        display_name: Some(case.label_base()),
        status: TestStatus::Failed,
        issues: vec![Issue {
            message,
            file: file.clone(),
            line,
        }],
        duration: std::time::Duration::ZERO,
        file,
        line,
        skip_reason: None,
    }
}

/// Run a single discovered test in a fresh session, converting its outcome and
/// recorded issues into a [`TestResult`]. A suite test constructs a fresh
/// instance (`Suite().method()`); a free test calls the function directly.
fn run_one(
    interp: &mut Interpreter<'_>,
    analysis: &'static Analysis,
    case: &TestCase,
    id: &str,
    label: Option<String>,
    call: tswift_frontend::Node<'static>,
) -> TestResult {
    session::begin();
    let start = Instant::now();
    let outcome = {
        let ctx: &mut dyn StdContext = interp;
        ctx.eval_node(&call)
    };
    let duration = start.elapsed();
    let (raw_issues, aborted) = session::end();

    let mut issues: Vec<Issue> = raw_issues
        .into_iter()
        .map(|raw| {
            // A raw line of 0 means "no source location" (e.g. `Issue.record`,
            // which is a static call with no node); attribute it to the test's
            // own declaration line rather than remapping the invalid line 0.
            let source_line = if raw.line == 0 { case.line } else { raw.line };
            let (file, line) = analysis.locate(source_line);
            Issue {
                message: raw.message,
                file,
                line,
            }
        })
        .collect();

    // An uncaught throw or a runtime trap that is *not* a `#require` abort is
    // itself a test failure.
    if let Err(err) = &outcome {
        if !aborted {
            let (file, line) = analysis.locate(case.line);
            issues.push(Issue {
                message: match err {
                    StdError::Throw(value) => format!("Test threw an error: {value}"),
                    StdError::Error(e) => format!("Test failed with a runtime error: {e}"),
                },
                file,
                line,
            });
        }
    }

    let status = if issues.is_empty() {
        TestStatus::Passed
    } else {
        TestStatus::Failed
    };
    let (file, line) = analysis.locate(case.line);
    TestResult {
        id: id.to_string(),
        display_name: label,
        status,
        issues,
        duration,
        file,
        line,
        skip_reason: None,
    }
}

/// Parse one synthetic driver statement per run into a single retained
/// analysis, returning the per-run call node (index-aligned with `lines`).
///
/// Every call is wrapped in `try await` so throwing and async tests run through
/// the same node without per-test codegen (both are transparent for
/// non-throwing / non-async bodies). Sema may flag the unresolved user symbols
/// in the driver, but the runtime resolves them against the already-loaded
/// program and we evaluate each statement node directly.
fn build_drivers(
    interp: &mut Interpreter<'_>,
    lines: &[String],
) -> Result<Vec<tswift_frontend::Node<'static>>, String> {
    let mut source = String::new();
    for line in lines {
        source.push_str(line);
        source.push('\n');
    }
    let driver = Analysis::analyze(&source, "<test-driver>")
        .map_err(|e| format!("failed to build test driver: {e}"))?;
    let driver: &'static Analysis = interp.retain_analysis(Rc::new(driver));
    drivers_from(driver, lines.len())
}

/// Validate a parsed driver against the expected test count and return one call
/// node per test. A driver that failed to parse (syntax error → empty root) or
/// whose statement count does not match the discovered tests is a hard build
/// failure: a missing call node must never be treated as a passing test.
fn drivers_from(
    driver: &'static Analysis,
    expected: usize,
) -> Result<Vec<tswift_frontend::Node<'static>>, String> {
    if !driver.is_ok() {
        let diags = driver
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| format!("  {}:{}: {}", d.line, d.col, d.message))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("failed to build test driver:\n{diags}"));
    }
    let statements: Vec<tswift_frontend::Node<'static>> = driver.root().children().collect();
    if statements.len() != expected {
        return Err(format!(
            "failed to build test driver: expected {expected} driver statement(s), parsed {}",
            statements.len()
        ));
    }
    Ok(statements)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> RunReport {
        run_tests(
            &[SourceFile::new("Tests.swift", src)],
            &RunOptions::default(),
        )
    }

    #[test]
    fn passing_test_reports_one_pass() {
        let report = run("@Test func t() { #expect(1 + 1 == 2) }\n");
        assert_eq!(report.passed(), 1);
        assert_eq!(report.failed(), 0);
        assert!(report.is_success());
    }

    fn leak(src: &str) -> &'static Analysis {
        Box::leak(Box::new(Analysis::analyze(src, "<d>").unwrap()))
    }

    #[test]
    fn driver_parse_failure_is_not_success() {
        // A driver that fails to parse must fail the whole build, never yield a
        // silently-passing (missing) call node.
        let driver = leak("@#!(\n");
        assert!(drivers_from(driver, 1).is_err());
    }

    #[test]
    fn driver_count_mismatch_is_error() {
        // Fewer parsed statements than discovered tests must be a hard error, so
        // no test is mapped to a `None`/`Void` "pass".
        let driver = leak("foo()\n");
        assert!(drivers_from(driver, 2).is_err());
    }

    #[test]
    fn expect_non_bool_operand_fails_hard() {
        let report = run("@Test func t() { #expect(\"hello\") }\n");
        assert_eq!(report.passed(), 0);
        assert_eq!(report.failed(), 1);
        assert!(report.tests[0].issues[0].message.contains("Bool"));
    }

    #[test]
    fn expect_outside_test_traps() {
        // A top-level `#expect` runs during load, before any session is open.
        let report = run("#expect(true)\n");
        assert!(!report.is_success());
        let err = report.compile_error.expect("top-level #expect must trap");
        assert!(err.to_string().contains("outside a test"));
    }

    #[test]
    fn expect_evaluates_impure_operand_once() {
        // `bump()` increments a global; a failing comparison must call it once,
        // so the captured detail shows `bump() → 1`, not a re-evaluated `→ 2`.
        let report = run(concat!(
            "var counter = 0\n",
            "func bump() -> Int { counter += 1; return counter }\n",
            "@Test func t() { #expect(bump() == 99) }\n",
        ));
        assert_eq!(report.failed(), 1);
        let message = &report.tests[0].issues[0].message;
        assert!(
            message.contains("bump() → 1"),
            "expected single evaluation, got: {message}"
        );
    }

    #[test]
    fn enabled_if_non_bool_condition_fails_not_skips() {
        // A broken predicate must FAIL the test with a clear issue, never
        // silently skip it (a skip stays CI-green).
        let report = run("@Test(.enabled(if: 1)) func t() {}\n");
        assert_eq!(report.passed(), 0);
        assert_eq!(report.skipped(), 0);
        assert_eq!(report.failed(), 1);
        let message = &report.tests[0].issues[0].message;
        assert!(
            message.contains("failed to evaluate .enabled(if:) condition"),
            "{message}"
        );
    }

    #[test]
    fn enabled_if_throwing_condition_fails_not_skips() {
        let report = run(concat!(
            "func explode() -> Bool { fatalError(\"boom\") }\n",
            "@Test(.enabled(if: explode())) func t() {}\n",
        ));
        assert_eq!(report.passed(), 0);
        assert_eq!(report.skipped(), 0);
        assert_eq!(report.failed(), 1);
        let message = &report.tests[0].issues[0].message;
        assert!(
            message.contains("failed to evaluate .enabled(if:) condition"),
            "{message}"
        );
    }

    #[test]
    fn enabled_if_true_condition_still_runs() {
        let report = run("@Test(.enabled(if: 1 > 0)) func t() { #expect(true) }\n");
        assert_eq!(report.passed(), 1);
    }

    #[test]
    fn enabled_if_false_condition_still_skips() {
        let report = run("@Test(.enabled(if: 1 > 2)) func t() { #expect(true) }\n");
        assert_eq!(report.skipped(), 1);
        assert_eq!(report.failed(), 0);
    }

    #[test]
    fn empty_arguments_array_is_visible_not_silent() {
        // `Expansion::Cases(vec![])` used to vanish into zero plans; it must
        // surface as a visible Skip, not disappear from the report entirely.
        let report = run("@Test(arguments: []) func p(x: Int) { #expect(true) }\n");
        assert_eq!(report.tests.len(), 1, "empty expansion must still report");
        assert_eq!(report.skipped(), 1);
        assert_eq!(
            report.tests[0].skip_reason.as_deref(),
            Some("no argument cases")
        );
    }

    #[test]
    fn empty_zip_factor_is_visible_not_silent() {
        let report =
            run("@Test(arguments: zip([1, 2], [])) func p(a: Int, b: Int) { #expect(true) }\n");
        assert_eq!(report.tests.len(), 1);
        assert_eq!(report.skipped(), 1);
    }

    #[test]
    fn empty_cartesian_factor_is_visible_not_silent() {
        let report = run("@Test(arguments: [1, 2], []) func p(x: Int, y: Int) { #expect(true) }\n");
        assert_eq!(report.tests.len(), 1);
        assert_eq!(report.skipped(), 1);
    }

    #[test]
    fn non_collection_arguments_fails_with_clear_message() {
        let report = run(concat!(
            "let bag = [1, 2]\n",
            "@Test(arguments: bag) func p(x: Int) { #expect(true) }\n",
        ));
        assert_eq!(report.failed(), 1);
        let message = &report.tests[0].issues[0].message;
        assert!(message.contains("unsupported arguments"), "{message}");
        assert!(
            message.contains("expected collection literal or zip"),
            "{message}"
        );
    }

    #[test]
    fn expect_failure_remaps_to_originating_file() {
        let report = run_tests(
            &[
                SourceFile::new("FileA.swift", "@Test func a() { #expect(true) }\n"),
                SourceFile::new("FileB.swift", "@Test func b() {\n  #expect(1 == 2)\n}\n"),
            ],
            &RunOptions::default(),
        );
        assert_eq!(report.failed(), 1);
        let failing = report
            .tests
            .iter()
            .find(|t| t.id == "b()")
            .expect("b() present");
        let issue = &failing.issues[0];
        assert_eq!(issue.file.as_deref(), Some("FileB.swift"));
        assert_eq!(issue.line, 2);
    }
}
