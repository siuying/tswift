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
mod render;
mod report;
mod session;

use std::rc::Rc;
use std::time::Instant;

use tswift_core::{Interpreter, StdContext, StdError, SwiftValue};
use tswift_frontend::{Analysis, SourceFile};

pub use discover::TestCase;
pub use report::{Issue, RunReport, TestResult, TestStatus};

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
                compile_error: Some(err.to_string()),
                ..RunReport::default()
            }
        }
    };
    if !analysis.is_ok() {
        let diags = analysis
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| {
                let file = d.file.as_deref().unwrap_or("<input>");
                format!("{file}:{}:{}: {}", d.line, d.col, d.message)
            })
            .collect::<Vec<_>>()
            .join("\n");
        return RunReport {
            compile_error: Some(diags),
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
            compile_error: Some(err.to_string()),
            ..RunReport::default()
        };
    }

    let mut cases = discover::discover(analysis.root());
    if let Some(filter) = &options.filter {
        cases.retain(|c| c.matches_filter(filter));
    }

    // Build one synthetic driver holding a call per test, retained (not leaked)
    // for the interpreter's lifetime; index i maps to cases[i].
    let driver_nodes = build_drivers(&mut interp, &cases);

    let run_start = Instant::now();
    let mut results = Vec::with_capacity(cases.len());
    for (case, node) in cases.iter().zip(driver_nodes) {
        results.push(run_one(&mut interp, analysis, case, node));
    }
    RunReport {
        tests: results,
        duration: run_start.elapsed(),
        compile_error: None,
    }
}

/// Run a single discovered test in a fresh session, converting its outcome and
/// recorded issues into a [`TestResult`]. A suite test constructs a fresh
/// instance (`Suite().method()`); a free test calls the function directly.
fn run_one(
    interp: &mut Interpreter<'_>,
    analysis: &'static Analysis,
    case: &TestCase,
    call: Option<tswift_frontend::Node<'static>>,
) -> TestResult {
    session::begin();
    let start = Instant::now();
    let outcome = match call {
        Some(node) => {
            let ctx: &mut dyn StdContext = interp;
            ctx.eval_node(&node)
        }
        None => Ok(SwiftValue::Void),
    };
    let duration = start.elapsed();
    let (raw_issues, aborted) = session::end();

    let mut issues: Vec<Issue> = raw_issues
        .into_iter()
        .map(|raw| {
            let (file, line) = analysis.locate(raw.line);
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
        id: case.id(),
        display_name: case.display_name.clone(),
        status,
        issues,
        duration,
        file,
        line,
    }
}

/// Parse one synthetic driver statement per test into a single retained
/// analysis, returning the per-test call node (index-aligned with `cases`).
///
/// Every call is wrapped in `try await` so throwing and async tests run through
/// the same node without per-test codegen (both are transparent for
/// non-throwing / non-async bodies). Sema may flag the unresolved user symbols
/// in the driver, but the runtime resolves them against the already-loaded
/// program and we evaluate each statement node directly.
fn build_drivers(
    interp: &mut Interpreter<'_>,
    cases: &[TestCase],
) -> Vec<Option<tswift_frontend::Node<'static>>> {
    let mut source = String::new();
    for case in cases {
        match &case.suite_type {
            Some(suite) => source.push_str(&format!("try await {suite}().{}()\n", case.func_name)),
            None => source.push_str(&format!("try await {}()\n", case.func_name)),
        }
    }
    let driver = match Analysis::analyze(&source, "<test-driver>") {
        Ok(driver) => driver,
        Err(_) => return vec![None; cases.len()],
    };
    let driver: &'static Analysis = interp.retain_analysis(Rc::new(driver));
    let statements: Vec<tswift_frontend::Node<'static>> = driver.root().children().collect();
    // Index alignment holds because we emit exactly one statement per case.
    (0..cases.len())
        .map(|i| statements.get(i).copied())
        .collect()
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
}
