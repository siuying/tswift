//! Test discovery *without running* — the platform-neutral list seam.
//!
//! [`list_tests`] analyzes a program and returns a [`TestDescriptor`] per
//! discovered `@Test` (one per test, not per parameterized case: the case
//! count is reported separately). It performs no evaluation, so a
//! `.enabled(if:)` condition (which needs the program loaded and run) is *not*
//! reflected in [`TestDescriptor::skipped`]; only the static `.disabled("…")`
//! trait is. Hosts (CLI table / web playground / iOS) render these descriptors
//! to let a user pick which tests to run before paying to run them.

use std::rc::Rc;

use tswift_core::Interpreter;
use tswift_frontend::{Analysis, SourceFile};

use crate::discover;
use crate::params::{self, Expansion};
use crate::report::CompileError;
use crate::traits::Trait;

/// A single discovered test, described for a host to render/select — the result
/// of discovery *without* running.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestDescriptor {
    /// Canonical id (`"free()"`, `"MathSuite/pass()"`), the exact string a
    /// host passes back in [`crate::RunOptions::ids`] to select this test.
    pub id: String,
    /// `@Test("…")` display name, if any.
    pub display_name: Option<String>,
    /// The owning suite's dotted type path (`"Outer.Inner"`), or `None` for a
    /// free test.
    pub suite_path: Option<String>,
    /// Originating source file, when known.
    pub file: Option<String>,
    /// 1-based declaration line in `file`.
    pub line: u32,
    /// Tag names on this test (its own `.tags(...)` plus inherited), in source
    /// order.
    pub tags: Vec<String>,
    /// Number of parameterized cases when cheaply known from a literal
    /// `arguments:` collection; `None` for a non-parameterized test or an
    /// `arguments:` shape that can't be expanded structurally.
    pub case_count: Option<usize>,
    /// The exact selectable id of every parameterized case (e.g. `"p() - 2"`),
    /// in case order — the same ids [`crate::run_tests`] assigns, so a host
    /// can pass one straight back in `RunOptions::ids`. Empty when
    /// [`case_count`](Self::case_count) is `None`.
    pub cases: Vec<String>,
    /// Statically skipped by a `.disabled(...)` trait (a `.enabled(if:)`
    /// condition is not evaluated during listing, so it never sets this).
    pub skipped: bool,
    /// The `.disabled("reason")` reason, when [`skipped`](Self::skipped).
    pub skip_reason: Option<String>,
    /// The owning test-target unit's name, set by a multi-target host (the
    /// `tswift test --list` CLI over a `Package.swift` with more than one
    /// `.testTarget`); `None` for a single-unit listing (a plain file/dir, or
    /// the wasm/FFI `listTests` seam, which never see multiple units).
    pub target: Option<String>,
}

/// Analyze `files` and describe every discovered `@Test` without running any.
///
/// `Err` on a compile error (never a silently empty list — a caller must be
/// able to tell "no tests" apart from "the program doesn't compile" and
/// render/report the failure accordingly, same as [`crate::run_tests`]);
/// discovery itself never evaluates code.
pub fn list_tests(files: &[SourceFile]) -> Result<Vec<TestDescriptor>, CompileError> {
    let analysis =
        Analysis::analyze_program(files).map_err(|err| CompileError::Message(err.to_string()))?;
    if !analysis.is_ok() {
        let diags: Vec<_> = analysis
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .cloned()
            .collect();
        return Err(CompileError::Diagnostics(diags));
    }
    // Retain the analysis for `Node<'static>` cursor lifetimes without a
    // permanent leak (dropped when this throwaway interpreter drops).
    let mut sink = std::io::sink();
    let mut interp = Interpreter::new(&mut sink);
    let analysis: &'static Analysis = interp.retain_analysis(Rc::new(analysis));

    let tests = discover::discover(analysis.root())
        .iter()
        .map(|case| {
            let (file, line) = analysis.locate(case.line);
            let (skipped, skip_reason) = disabled_reason(case);
            let (case_count, cases) = match params::expand(&case.node) {
                Expansion::Cases(rows) => {
                    let suffixes = params::case_id_suffixes(&rows);
                    let ids = suffixes
                        .iter()
                        .map(|suffix| format!("{}{suffix}", case.id()))
                        .collect();
                    (Some(rows.len()), ids)
                }
                Expansion::None | Expansion::Unsupported(_) => (None, Vec::new()),
            };
            TestDescriptor {
                id: case.id(),
                display_name: case.display_name.clone(),
                suite_path: case.suite_type.clone(),
                file,
                line,
                tags: case.tags(),
                case_count,
                cases,
                skipped,
                skip_reason,
                target: None,
            }
        })
        .collect();
    Ok(tests)
}

/// The `.disabled(...)` skip status of a test, read statically (no evaluation).
fn disabled_reason(case: &discover::TestCase) -> (bool, Option<String>) {
    for trait_ in &case.traits {
        if let Trait::Disabled(reason) = trait_ {
            return (true, reason.clone());
        }
    }
    (false, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(src: &str) -> Vec<TestDescriptor> {
        list_tests(&[SourceFile::new("Tests.swift", src)]).expect("compiles")
    }

    #[test]
    fn lists_free_test_with_id_and_location() {
        let tests = list("@Test(\"adds\")\nfunc addition() {}\n");
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].id, "addition()");
        assert_eq!(tests[0].display_name.as_deref(), Some("adds"));
        assert_eq!(tests[0].file.as_deref(), Some("Tests.swift"));
        assert_eq!(tests[0].line, 2);
        assert!(tests[0].suite_path.is_none());
    }

    #[test]
    fn lists_suite_test_with_suite_path() {
        let tests = list("struct MathSuite {\n  @Test func pass() {}\n}\n");
        assert_eq!(tests[0].id, "MathSuite/pass()");
        assert_eq!(tests[0].suite_path.as_deref(), Some("MathSuite"));
    }

    #[test]
    fn reports_parameterized_case_count() {
        let tests = list("@Test(arguments: [1, 2, 3]) func p(x: Int) {}\n");
        assert_eq!(tests[0].case_count, Some(3));
    }

    #[test]
    fn non_parameterized_has_no_case_count() {
        let tests = list("@Test func t() {}\n");
        assert_eq!(tests[0].case_count, None);
    }

    #[test]
    fn reports_disabled_skip_without_running() {
        let tests = list("@Test(.disabled(\"flaky\")) func t() {}\n");
        assert!(tests[0].skipped);
        assert_eq!(tests[0].skip_reason.as_deref(), Some("flaky"));
    }

    #[test]
    fn enabled_if_is_not_evaluated_during_listing() {
        // `.enabled(if:)` needs the program run; listing must not evaluate it,
        // so the test is reported as not-statically-skipped.
        let tests = list("@Test(.enabled(if: false)) func t() {}\n");
        assert!(!tests[0].skipped);
    }

    #[test]
    fn carries_tags() {
        let tests = list("@Test(.tags(.fast)) func t() {}\n");
        assert_eq!(tests[0].tags, vec!["fast".to_string()]);
    }

    #[test]
    fn compile_error_is_surfaced_not_silently_empty() {
        // A listing that can't compile must say so — never a silently empty
        // (indistinguishable-from-"no tests") list.
        let err = list_tests(&[SourceFile::new("Tests.swift", "@Test func broken( {}\n")])
            .expect_err("compile error must surface");
        assert!(err.to_string().contains("Tests.swift"), "{err}");
    }

    #[test]
    fn reports_parameterized_case_ids() {
        let tests = list("@Test(arguments: [1, 2, 3]) func p(x: Int) {}\n");
        assert_eq!(
            tests[0].cases,
            vec![
                "p() - 1".to_string(),
                "p() - 2".to_string(),
                "p() - 3".to_string()
            ]
        );
    }

    #[test]
    fn non_parameterized_has_no_case_ids() {
        let tests = list("@Test func t() {}\n");
        assert!(tests[0].cases.is_empty());
    }

    #[test]
    fn duplicate_argument_case_ids_are_disambiguated() {
        let tests = list("@Test(arguments: [1, 1]) func p(x: Int) {}\n");
        assert_eq!(
            tests[0].cases,
            vec!["p() - 1 (#1)".to_string(), "p() - 1 (#2)".to_string()]
        );
    }

    #[test]
    fn target_defaults_to_none() {
        let tests = list("@Test func t() {}\n");
        assert!(tests[0].target.is_none());
    }
}
