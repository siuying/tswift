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

mod descriptor;
mod discover;
mod expect;
mod params;
mod render;
mod report;
mod session;
mod traits;
mod wire;

use std::collections::HashSet;
use std::rc::Rc;
use std::time::Instant;

use tswift_core::{Interpreter, StdContext, StdError, SwiftValue};
use tswift_frontend::{Analysis, SourceFile};

use traits::Trait;

pub use descriptor::{list_tests, TestDescriptor};
pub use discover::TestCase;
pub use report::{CompileError, Issue, RunReport, TestResult, TestStatus};
pub use wire::{descriptors_to_json, error_json, parse_run_options, report_to_json};

/// Options controlling a test run.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Case-sensitive substring filter on a test's id / display name; `None`
    /// runs every test. Ignored when [`ids`](Self::ids) is set (exact
    /// selection takes precedence over the substring filter).
    pub filter: Option<String>,
    /// Exact canonical-id selection (distinct from the substring `filter`):
    /// `Some(["MathSuite/pass()", "p() - 2"])` runs only the listed tests.
    /// A base test id (`"p()"` — the base id carries no parameter labels)
    /// selects every one of its parameterized cases; an exact case id (e.g.
    /// `"p() - 2"`, argument value suffixed) selects just that case. An id
    /// matching no discovered test is an error (the run reports the unknown
    /// ids, never a silent zero-tests success). `None` selects everything.
    pub ids: Option<Vec<String>>,
}

/// Register the `Testing` module surface (`#expect`, `#require`) on `interp`.
///
/// Mirrors `tswift_swiftdata::install`: the macros are scoped to the `Testing`
/// module so strict import-gating resolves them only after `import Testing`.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.module("Testing", |interp| {
        interp.register_macro("expect", expect::expect_macro);
        interp.register_macro("require", expect::require_macro);
        interp.register_free_fn("withKnownIssue", expect::with_known_issue);
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
    // Exact-id selection (`ids`) takes precedence over the substring `filter`;
    // the filter only applies when no explicit id selection is given.
    if options.ids.is_none() {
        if let Some(filter) = &options.filter {
            cases.retain(|c| c.matches_filter(filter));
        }
    }

    // Resolve each test into a flat list of plans — a skip, or one run per
    // parameterized case (a single run for an ordinary test). Only runs need a
    // synthetic driver call, so `.enabled(if:)` conditions are evaluated here
    // (against the loaded program) before any driver is built.
    let mut plans: Vec<Plan> = cases
        .iter()
        .flat_map(|c| plan_case(&mut interp, c))
        .collect();

    // Apply exact-id selection. An id may name a whole test (`case.id()`, which
    // selects all its parameterized cases) or one expanded case (a plan's own
    // id). Any selection id matching neither is an error listing the unknowns —
    // never a silent zero-tests success.
    if let Some(ids) = &options.ids {
        let mut known: HashSet<String> = cases.iter().map(|c| c.id()).collect();
        for plan in &plans {
            known.insert(plan.selection_id());
        }
        let unknown: Vec<String> = ids
            .iter()
            .filter(|id| !known.contains(id.as_str()))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            return RunReport {
                compile_error: Some(CompileError::Message(format!(
                    "unknown test id(s): {}",
                    unknown.join(", ")
                ))),
                ..RunReport::default()
            };
        }
        let selection: HashSet<&str> = ids.iter().map(String::as_str).collect();
        plans.retain(|plan| {
            selection.contains(plan.case().id().as_str())
                || selection.contains(plan.selection_id().as_str())
        });
    }
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

impl<'a> Plan<'a> {
    /// The discovered test this plan belongs to.
    fn case(&self) -> &'a TestCase {
        match self {
            Plan::Skip { case, .. } | Plan::Fail { case, .. } | Plan::Run { case, .. } => case,
        }
    }

    /// The most specific id a host can select this plan by: a run's per-case id
    /// (`"p(x:) - 2"`), or the whole test's id for a skip/fail plan.
    fn selection_id(&self) -> String {
        match self {
            Plan::Run { id, .. } => id.clone(),
            Plan::Skip { case, .. } | Plan::Fail { case, .. } => case.id(),
        }
    }
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
        params::Expansion::Cases(rows) => {
            let name = case
                .display_name
                .clone()
                .unwrap_or_else(|| params::signature(&case.node, &case.func_name));
            // Qualify with the suite when there's no display name to fall
            // back on: `suite_display` (an `@Suite("…")` name) if set, else
            // the suite's type path itself, matching `label_base()`/`id()`
            // (a suite test's label must never lose its qualifying type).
            let base = match (&case.suite_display, &case.suite_type) {
                (Some(suite), _) => format!("{suite}/{name}"),
                (None, Some(suite)) => format!("{}/{name}", suite.replace('.', "/")),
                (None, None) => name,
            };
            let rendered: Vec<String> = rows
                .iter()
                .map(|row| row.iter().map(render::expr).collect::<Vec<_>>().join(", "))
                .collect();
            // Disambiguate duplicate-argument cases (e.g. `arguments: [1, 1]`)
            // with a 1-based occurrence suffix so no two labels collide.
            let mut total_occurrences: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::new();
            for args in &rendered {
                *total_occurrences.entry(args.as_str()).or_insert(0) += 1;
            }
            let mut seen_so_far: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::new();
            rendered
                .iter()
                .map(|args| {
                    let suffix = if total_occurrences[args.as_str()] > 1 {
                        let n = seen_so_far.entry(args.as_str()).or_insert(0);
                        *n += 1;
                        format!(" (#{n})")
                    } else {
                        String::new()
                    };
                    Plan::Run {
                        case,
                        id: format!("{} - {args}{suffix}", case.id()),
                        label: Some(format!("{base} - {args}{suffix}")),
                        driver: driver_line(case, args),
                    }
                })
                .collect()
        }
    }
}

/// The `try await`-wrapped driver statement that invokes `case` with `args`
/// (a comma-separated argument source, empty for a no-argument test).
///
/// A suite test binds a fresh instance to a mutable local inside a `do` block
/// (`do { var __suite = Suite(); try await __suite.method(args) }`) rather than
/// calling on a bare temporary (`Suite().method()`): the local is released when
/// the block ends, so a `class` suite's `deinit` runs deterministically after
/// each test (expression temporaries are not ARC-released by the interpreter,
/// so the bare-temporary form would never fire `deinit`). It is bound `var` so
/// a `struct` suite's mutating test method can update `self`; a `struct` suite
/// has no `deinit`, so the block form is otherwise a harmless no-op for it.
fn driver_line(case: &TestCase, args: &str) -> String {
    match &case.suite_type {
        Some(suite) => format!(
            "do {{ var __suite = try await {suite}(); try await __suite.{}({args}) }}",
            case.func_name
        ),
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
            // Annotation-only traits never affect the skip decision.
            Trait::Tags(_) | Trait::Bug(_) | Trait::TimeLimit(_) => {}
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
        bugs: case.bugs(),
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
            known: false,
        }],
        duration: std::time::Duration::ZERO,
        file,
        line,
        skip_reason: None,
        bugs: case.bugs(),
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
                known: raw.known,
            }
        })
        .collect();

    // Soft time-limit check (plan §1.2): the runner never hard-kills a test, it
    // measures the elapsed time and records a real (non-known) issue when a
    // `.timeLimit(…)` was exceeded.
    if let Some(limit) = case.time_limit() {
        if duration > limit {
            let (file, line) = analysis.locate(case.line);
            issues.push(Issue {
                message: format!(
                    "Time limit exceeded: test took {:.3}s, limit was {:.3}s",
                    duration.as_secs_f64(),
                    limit.as_secs_f64()
                ),
                file,
                line,
                known: false,
            });
        }
    }

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
                known: false,
            });
        }
    }

    // Only non-known issues fail a test; a test whose sole issues are known
    // (from `withKnownIssue`) still passes, with those issues reported.
    let status = if issues.iter().any(|i| !i.known) {
        TestStatus::Failed
    } else {
        TestStatus::Passed
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
        bugs: case.bugs(),
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

    fn run_ids(src: &str, ids: &[&str]) -> RunReport {
        run_tests(
            &[SourceFile::new("Tests.swift", src)],
            &RunOptions {
                filter: None,
                ids: Some(ids.iter().map(|s| s.to_string()).collect()),
            },
        )
    }

    #[test]
    fn ids_selection_runs_only_named_tests() {
        let report = run_ids(
            "@Test func a() { #expect(true) }\n@Test func b() { #expect(true) }\n",
            &["a()"],
        );
        assert_eq!(report.tests.len(), 1);
        assert_eq!(report.tests[0].id, "a()");
    }

    #[test]
    fn ids_selection_of_parameterized_base_runs_all_cases() {
        let report = run_ids(
            "@Test(arguments: [1, 2, 3]) func p(x: Int) { #expect(x > 0) }\n",
            &["p()"],
        );
        assert_eq!(report.tests.len(), 3);
    }

    #[test]
    fn ids_selection_of_exact_case_runs_one_case() {
        let report = run_ids(
            "@Test(arguments: [1, 2, 3]) func p(x: Int) { #expect(x > 0) }\n",
            &["p() - 2"],
        );
        assert_eq!(report.tests.len(), 1);
        assert!(report.tests[0].id.contains("- 2"), "{}", report.tests[0].id);
    }

    #[test]
    fn unknown_id_is_an_error_not_silent_zero() {
        let report = run_ids("@Test func a() {}\n", &["a()", "missing()"]);
        assert!(!report.is_success());
        let err = report.compile_error.expect("unknown id must error");
        assert!(err.to_string().contains("missing()"), "{err}");
        assert!(report.tests.is_empty(), "no tests run on unknown id");
    }

    #[test]
    fn ids_selection_takes_precedence_over_filter() {
        let report = run_tests(
            &[SourceFile::new(
                "Tests.swift",
                "@Test func alpha() { #expect(true) }\n@Test func beta() { #expect(true) }\n",
            )],
            &RunOptions {
                filter: Some("beta".into()),
                ids: Some(vec!["alpha()".into()]),
            },
        );
        assert_eq!(report.tests.len(), 1);
        assert_eq!(report.tests[0].id, "alpha()");
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
    fn parameterized_suite_test_label_includes_suite_qualifier() {
        // No `@Test`/`@Suite` display names: the parameterized label must
        // still carry the suite qualifier, matching `label_base()`/`id()`.
        let src = "struct MathSuite {\n  @Test(arguments: [1, 2]) func p(x: Int) {}\n}\n";
        let report = run(src);
        assert_eq!(report.tests.len(), 2);
        for t in &report.tests {
            let label = t.label();
            assert!(
                label.starts_with("MathSuite/"),
                "expected suite-qualified label, got: {label}"
            );
        }
    }

    #[test]
    fn duplicate_argument_values_get_disambiguated_labels() {
        let report = run("@Test(arguments: [1, 1]) func p(x: Int) { #expect(true) }\n");
        assert_eq!(report.tests.len(), 2);
        let labels: std::collections::HashSet<&str> =
            report.tests.iter().map(|t| t.label()).collect();
        assert_eq!(labels.len(), 2, "duplicate-argument labels must be unique");
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
