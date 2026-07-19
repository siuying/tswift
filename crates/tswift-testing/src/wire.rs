//! JSON wire format for the platform-neutral test seam.
//!
//! [`TestDescriptor`] (`descriptor.rs`), [`RunReport`]/[`TestResult`]/[`Issue`]
//! (`report.rs`), and [`RunOptions`] (`lib.rs`) derive `serde::Serialize`
//! (and `Deserialize` for `RunOptions`, the one type actually parsed from an
//! untrusted document) directly, `#[serde(rename_all = "camelCase")]` where a
//! field name needs it, so this module is a thin `serde_json` wrapper rather
//! than hand-building [`tswift_core::json::Json`] trees. `serde`/`serde_json`
//! are already vendored transitively (via `criterion`, a dev-dependency) and
//! their exact locked versions are pinned in the workspace manifest, so this
//! is offline-safe (`docs/agents/environment.md`); `tswift_core::json` stays
//! the shared layer for everything *outside* this crate (e.g.
//! `tswift_frontend::symbols::to_json`), which this change does not touch.
//!
//! - [`descriptors_to_json`] serializes a [`TestDescriptor`] list (the
//!   `listTests` / `tswift_list_tests` response, and a successful
//!   `tswift test --list --json`).
//! - [`list_result_to_json`] / [`list_units_to_json`] wrap a [`list_tests`]
//!   [`Result`] (or several, one per test-target unit) so a compile error
//!   during listing is reported structurally instead of silently
//!   disappearing into an empty list.
//! - [`report_to_json`] serializes a [`RunReport`] (the `runTests` /
//!   `tswift_run_tests` response).
//! - [`parse_run_options`] decodes the `{"filter":…,"ids":[…]}` options object
//!   the wasm/FFI `runTests` entry points accept.
//!
//! ## The two `"ok":false` shapes
//!
//! Two distinct failure envelopes exist, both `{"ok":false,…}` but otherwise
//! different, because they answer different questions:
//!
//! - **The request parsed, but the *program* doesn't compile** — a
//!   [`list_tests`]/[`crate::run_tests`] `Err`/`compile_error`. Carries
//!   `"compileError":"<message>"` (plus `"tests":[]}`/a zeroed report), from
//!   [`list_result_to_json`], [`list_units_to_json`], and [`report_to_json`]
//!   ([`RunReport`]'s own `Serialize` impl).
//! - **The request itself was malformed** (bad `module_json`, before any
//!   program analysis ran) — [`error_json`]'s envelope, carrying
//!   `"error":"<message>"` instead. There is no [`CompileError`]/`Analysis`
//!   behind this one, just a bare message.
//!
//! A caller distinguishing the two must check which key is present
//! (`compileError` vs. `error`), not just `ok`.

use serde_json::{json, Value};

use crate::descriptor::TestDescriptor;
use crate::report::{CompileError, RunReport};
use crate::RunOptions;

/// Serialize test descriptors as `{"ok":true,"tests":[…]}`.
pub fn descriptors_to_json(tests: &[TestDescriptor]) -> String {
    serde_json::to_string(&json!({ "ok": true, "tests": tests }))
        .expect("TestDescriptor serialization is infallible")
}

/// Serialize one [`list_tests`] outcome: `Ok` as [`descriptors_to_json`];
/// `Err` as `{"ok":false,"compileError":"…","tests":[]}` (see the module docs
/// for how this compares to [`error_json`]'s shape).
pub fn list_result_to_json(result: &Result<Vec<TestDescriptor>, CompileError>) -> String {
    match result {
        Ok(tests) => descriptors_to_json(tests),
        Err(err) => serde_json::to_string(&json!({
            "ok": false,
            "compileError": err.to_string(),
            "tests": Vec::<TestDescriptor>::new(),
        }))
        .expect("compile-error list serialization is infallible"),
    }
}

/// Combine several named [`list_tests`] outcomes — one per test-target unit,
/// as `tswift test --list --json` produces over a multi-`.testTarget`
/// package — into a single document: `tests` collects every
/// successfully-listed unit's descriptors (each already carrying its
/// `target`, set by the caller before passing it in here); `ok` is `false`
/// iff any unit failed to list, in which case `compileError` joins each
/// failing unit's message, prefixed by its (non-empty) target name so a
/// multi-unit failure names which unit broke.
pub fn list_units_to_json(units: &[(String, Result<Vec<TestDescriptor>, CompileError>)]) -> String {
    let mut tests: Vec<&TestDescriptor> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for (name, result) in units {
        match result {
            Ok(list) => tests.extend(list.iter()),
            Err(err) => errors.push(if name.is_empty() {
                err.to_string()
            } else {
                format!("{name}: {err}")
            }),
        }
    }
    let ok = errors.is_empty();
    let mut doc = json!({ "ok": ok, "tests": tests });
    if !ok {
        doc["compileError"] = Value::String(errors.join("\n"));
    }
    serde_json::to_string(&doc).expect("list-units serialization is infallible")
}

/// Serialize a run report. [`RunReport`] hand-writes its own `Serialize`
/// (see its doc comment) to compute `ok`/`passed`/`failed`/`skipped`/
/// `issueCount`/`durationMs`/`compileError` from methods rather than stored
/// fields; this function is just the `serde_json` call.
pub fn report_to_json(report: &RunReport) -> String {
    serde_json::to_string(report).expect("RunReport serialization is infallible")
}

/// Decode a `{"filter":"…","ids":["…",…]}` options object. A missing/`null`
/// field leaves the corresponding option unset; an unparseable document yields
/// [`RunOptions::default`] (run everything) so a host is never silently blocked.
pub fn parse_run_options(options_json: &str) -> RunOptions {
    serde_json::from_str(options_json).unwrap_or_default()
}

/// An `{"ok":false,"error":"…","tests":[]}` envelope for a malformed request
/// (e.g. `module_json` itself failed to parse, before any `Analysis` ran) —
/// see the module docs for how this differs from [`list_result_to_json`]'s
/// `compileError` shape.
pub fn error_json(message: &str) -> String {
    serde_json::to_string(&json!({
        "ok": false,
        "error": message,
        "tests": Vec::<TestDescriptor>::new(),
    }))
    .expect("error-envelope serialization is infallible")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::list_tests;
    use crate::report::{Issue, TestResult, TestStatus};
    use std::time::Duration;
    use tswift_frontend::SourceFile;

    #[test]
    fn descriptors_json_carries_id_and_case_count() {
        let tests = list_tests(&[SourceFile::new(
            "T.swift",
            "@Test(arguments: [1, 2]) func p(x: Int) {}\n",
        )])
        .expect("compiles");
        let out = descriptors_to_json(&tests);
        assert!(out.contains("\"ok\":true"), "{out}");
        assert!(out.contains("\"id\":\"p()\""), "{out}");
        assert!(out.contains("\"caseCount\":2"), "{out}");
        assert!(out.contains("\"cases\":[\"p() - 1\",\"p() - 2\"]"), "{out}");
    }

    #[test]
    fn list_result_json_surfaces_compile_error() {
        let result = list_tests(&[SourceFile::new("T.swift", "@Test func broken( {}\n")]);
        let out = list_result_to_json(&result);
        assert!(out.contains("\"ok\":false"), "{out}");
        assert!(out.contains("\"compileError\":"), "{out}");
        assert!(out.contains("\"tests\":[]"), "{out}");
    }

    #[test]
    fn list_units_json_skips_no_units_and_reports_ok() {
        let a = list_tests(&[SourceFile::new("A.swift", "@Test func a() {}\n")]);
        let out = list_units_to_json(&[("Unit".to_string(), a)]);
        assert!(out.contains("\"ok\":true"), "{out}");
        assert!(!out.contains("compileError"), "{out}");
    }

    #[test]
    fn list_units_json_names_failing_unit() {
        let broken = list_tests(&[SourceFile::new("B.swift", "@Test func broken( {}\n")]);
        let out = list_units_to_json(&[("BrokenUnit".to_string(), broken)]);
        assert!(out.contains("\"ok\":false"), "{out}");
        assert!(out.contains("BrokenUnit:"), "{out}");
    }

    #[test]
    fn report_json_carries_status_and_counts() {
        let report = crate::run_tests(
            &[SourceFile::new(
                "T.swift",
                "@Test func t() { #expect(1 == 2) }\n",
            )],
            &RunOptions::default(),
        );
        let out = report_to_json(&report);
        assert!(out.contains("\"ok\":false"), "{out}");
        assert!(out.contains("\"failed\":1"), "{out}");
        assert!(out.contains("\"status\":\"failed\""), "{out}");
        assert!(out.contains("1 == 2"), "{out}");
    }

    #[test]
    fn parse_options_reads_filter_and_ids() {
        let opts = parse_run_options("{\"filter\":\"a\",\"ids\":[\"t()\",\"s/u()\"]}");
        assert_eq!(opts.filter.as_deref(), Some("a"));
        assert_eq!(opts.ids, Some(vec!["t()".to_string(), "s/u()".to_string()]));
    }

    #[test]
    fn parse_options_tolerates_garbage() {
        let opts = parse_run_options("not json");
        assert!(opts.filter.is_none());
        assert!(opts.ids.is_none());
    }

    /// Rebuild the pre-serde `descriptors_to_json`/`report_to_json` output by
    /// hand (frozen copies of the old `tswift_core::json`-based
    /// implementation) and compare, as parsed [`serde_json::Value`]s, against
    /// today's serde-derived output — proving the schema didn't shift under
    /// the refactor (new fields — `cases`, `target` — are the one documented
    /// exception, stripped before comparing).
    mod schema_stability {
        use super::*;
        use tswift_core::json::{self, Json};

        fn old_descriptor_json(t: &TestDescriptor) -> Json {
            Json::Object(vec![
                ("id".into(), Json::Str(t.id.clone())),
                ("displayName".into(), old_opt_str(&t.display_name)),
                ("suitePath".into(), old_opt_str(&t.suite_path)),
                ("file".into(), old_opt_str(&t.file)),
                ("line".into(), Json::Int(t.line as i64)),
                (
                    "tags".into(),
                    Json::Array(t.tags.iter().cloned().map(Json::Str).collect()),
                ),
                (
                    "caseCount".into(),
                    t.case_count.map_or(Json::Null, |n| Json::Int(n as i64)),
                ),
                ("skipped".into(), Json::Bool(t.skipped)),
                ("skipReason".into(), old_opt_str(&t.skip_reason)),
            ])
        }

        fn old_opt_str(value: &Option<String>) -> Json {
            match value {
                Some(s) => Json::Str(s.clone()),
                None => Json::Null,
            }
        }

        fn old_descriptors_to_json(tests: &[TestDescriptor]) -> String {
            let items = tests.iter().map(old_descriptor_json).collect();
            json::to_string(&Json::Object(vec![
                ("ok".into(), Json::Bool(true)),
                ("tests".into(), Json::Array(items)),
            ]))
        }

        fn old_result_json(t: &TestResult) -> Json {
            Json::Object(vec![
                ("id".into(), Json::Str(t.id.clone())),
                ("displayName".into(), old_opt_str(&t.display_name)),
                ("status".into(), Json::Str(old_status_name(t.status).into())),
                ("file".into(), old_opt_str(&t.file)),
                ("line".into(), Json::Int(t.line as i64)),
                (
                    "durationMs".into(),
                    Json::Int(t.duration.as_millis() as i64),
                ),
                ("skipReason".into(), old_opt_str(&t.skip_reason)),
                (
                    "issues".into(),
                    Json::Array(
                        t.issues
                            .iter()
                            .map(|i| {
                                Json::Object(vec![
                                    ("message".into(), Json::Str(i.message.clone())),
                                    ("file".into(), old_opt_str(&i.file)),
                                    ("line".into(), Json::Int(i.line as i64)),
                                    ("known".into(), Json::Bool(i.known)),
                                ])
                            })
                            .collect(),
                    ),
                ),
                (
                    "bugs".into(),
                    Json::Array(t.bugs.iter().cloned().map(Json::Str).collect()),
                ),
            ])
        }

        fn old_status_name(status: TestStatus) -> &'static str {
            match status {
                TestStatus::Passed => "passed",
                TestStatus::Failed => "failed",
                TestStatus::Skipped => "skipped",
            }
        }

        fn old_report_to_json(report: &RunReport) -> String {
            let mut fields = vec![
                ("ok".into(), Json::Bool(report.is_success())),
                ("passed".into(), Json::Int(report.passed() as i64)),
                ("failed".into(), Json::Int(report.failed() as i64)),
                ("skipped".into(), Json::Int(report.skipped() as i64)),
                ("issueCount".into(), Json::Int(report.issue_count() as i64)),
                (
                    "durationMs".into(),
                    Json::Int(report.duration.as_millis() as i64),
                ),
                (
                    "compileError".into(),
                    match &report.compile_error {
                        Some(err) => Json::Str(err.to_string()),
                        None => Json::Null,
                    },
                ),
            ];
            fields.push((
                "tests".into(),
                Json::Array(report.tests.iter().map(old_result_json).collect()),
            ));
            json::to_string(&Json::Object(fields))
        }

        fn strip_new_descriptor_fields(mut value: Value) -> Value {
            if let Some(tests) = value.get_mut("tests").and_then(|v| v.as_array_mut()) {
                for test in tests {
                    if let Some(obj) = test.as_object_mut() {
                        obj.remove("cases");
                        obj.remove("target");
                    }
                }
            }
            value
        }

        #[test]
        fn descriptors_schema_matches_pre_serde_shape() {
            let tests = list_tests(&[SourceFile::new(
                "T.swift",
                "@Test(\"disp\") struct S { @Test(arguments: [1, 1]) func p(x: Int) {} }\n@Test func t() {}\n",
            )])
            .expect("compiles");
            let old: Value = serde_json::from_str(&old_descriptors_to_json(&tests)).unwrap();
            let new: Value = strip_new_descriptor_fields(
                serde_json::from_str(&descriptors_to_json(&tests)).unwrap(),
            );
            assert_eq!(old, new);
        }

        #[test]
        fn report_schema_matches_pre_serde_shape() {
            let result = TestResult {
                id: "b()".to_string(),
                display_name: Some("named".to_string()),
                status: TestStatus::Failed,
                issues: vec![Issue {
                    message: "boom".to_string(),
                    file: Some("t.swift".to_string()),
                    line: 3,
                    known: false,
                }],
                duration: Duration::from_millis(5),
                file: Some("t.swift".to_string()),
                line: 1,
                skip_reason: None,
                bugs: vec!["FB123".to_string()],
            };
            let report = RunReport {
                tests: vec![result],
                duration: Duration::from_millis(5),
                compile_error: None,
            };
            let old: Value = serde_json::from_str(&old_report_to_json(&report)).unwrap();
            let new: Value = serde_json::from_str(&report_to_json(&report)).unwrap();
            assert_eq!(old, new);
        }

        #[test]
        fn report_schema_matches_pre_serde_shape_with_compile_error() {
            let report = RunReport {
                tests: Vec::new(),
                duration: Duration::ZERO,
                compile_error: Some(CompileError::Message("boom".to_string())),
            };
            let old: Value = serde_json::from_str(&old_report_to_json(&report)).unwrap();
            let new: Value = serde_json::from_str(&report_to_json(&report)).unwrap();
            assert_eq!(old, new);
        }
    }
}
