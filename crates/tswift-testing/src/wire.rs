//! JSON wire format for the platform-neutral test seam.
//!
//! The workspace deliberately avoids `serde` (offline-build constraint,
//! `AGENTS.md`); like `tswift_frontend::symbols::to_json`, this module builds
//! JSON with the hand-rolled [`tswift_core::json`] layer so the wasm and FFI
//! hosts share one wire contract with the CLI.
//!
//! - [`descriptors_to_json`] serializes a [`TestDescriptor`] list (the
//!   `listTests` / `tswift_list_tests` response).
//! - [`report_to_json`] serializes a [`RunReport`] (the `runTests` /
//!   `tswift_run_tests` response).
//! - [`parse_run_options`] decodes the `{"filter":…,"ids":[…]}` options object
//!   the wasm/FFI `runTests` entry points accept.

use tswift_core::json::{self, Json};

use crate::descriptor::TestDescriptor;
use crate::report::{RunReport, TestResult, TestStatus};
use crate::RunOptions;

/// Serialize test descriptors as `{"ok":true,"tests":[…]}`.
pub fn descriptors_to_json(tests: &[TestDescriptor]) -> String {
    let items = tests.iter().map(descriptor_json).collect();
    json::to_string(&Json::Object(vec![
        ("ok".into(), Json::Bool(true)),
        ("tests".into(), Json::Array(items)),
    ]))
}

fn descriptor_json(t: &TestDescriptor) -> Json {
    Json::Object(vec![
        ("id".into(), Json::Str(t.id.clone())),
        ("displayName".into(), opt_str(&t.display_name)),
        ("suitePath".into(), opt_str(&t.suite_path)),
        ("file".into(), opt_str(&t.file)),
        ("line".into(), Json::Int(t.line as i64)),
        (
            "tags".into(),
            Json::Array(t.tags.iter().cloned().map(Json::Str).collect()),
        ),
        (
            "caseCount".into(),
            t.case_count
                .map_or(Json::Null, |n| Json::Int(n as i64)),
        ),
        ("skipped".into(), Json::Bool(t.skipped)),
        ("skipReason".into(), opt_str(&t.skip_reason)),
    ])
}

/// Serialize a run report as `{"ok":bool,"passed":…,"tests":[…]}`.
pub fn report_to_json(report: &RunReport) -> String {
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
        Json::Array(report.tests.iter().map(result_json).collect()),
    ));
    json::to_string(&Json::Object(fields))
}

fn result_json(t: &TestResult) -> Json {
    Json::Object(vec![
        ("id".into(), Json::Str(t.id.clone())),
        (
            "displayName".into(),
            opt_str(&t.display_name),
        ),
        ("status".into(), Json::Str(status_name(t.status).into())),
        ("file".into(), opt_str(&t.file)),
        ("line".into(), Json::Int(t.line as i64)),
        (
            "durationMs".into(),
            Json::Int(t.duration.as_millis() as i64),
        ),
        ("skipReason".into(), opt_str(&t.skip_reason)),
        (
            "issues".into(),
            Json::Array(
                t.issues
                    .iter()
                    .map(|i| {
                        Json::Object(vec![
                            ("message".into(), Json::Str(i.message.clone())),
                            ("file".into(), opt_str(&i.file)),
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

fn status_name(status: TestStatus) -> &'static str {
    match status {
        TestStatus::Passed => "passed",
        TestStatus::Failed => "failed",
        TestStatus::Skipped => "skipped",
    }
}

fn opt_str(value: &Option<String>) -> Json {
    match value {
        Some(s) => Json::Str(s.clone()),
        None => Json::Null,
    }
}

/// Decode a `{"filter":"…","ids":["…",…]}` options object. A missing/`null`
/// field leaves the corresponding option unset; an unparseable document yields
/// [`RunOptions::default`] (run everything) so a host is never silently blocked.
pub fn parse_run_options(options_json: &str) -> RunOptions {
    let Ok(root) = json::parse(options_json) else {
        return RunOptions::default();
    };
    let filter = match root.get("filter") {
        Some(Json::Str(s)) => Some(s.clone()),
        _ => None,
    };
    let ids = match root.get("ids") {
        Some(Json::Array(items)) => Some(
            items
                .iter()
                .filter_map(|v| match v {
                    Json::Str(s) => Some(s.clone()),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    };
    RunOptions { filter, ids }
}

/// An `{"ok":false,"error":"…","tests":[]}` envelope for a malformed request,
/// matching the shape of a successful [`descriptors_to_json`] response.
pub fn error_json(message: &str) -> String {
    json::to_string(&Json::Object(vec![
        ("ok".into(), Json::Bool(false)),
        ("error".into(), Json::Str(message.into())),
        ("tests".into(), Json::Array(Vec::new())),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::list_tests;
    use tswift_frontend::SourceFile;

    #[test]
    fn descriptors_json_carries_id_and_case_count() {
        let tests = list_tests(&[SourceFile::new(
            "T.swift",
            "@Test(arguments: [1, 2]) func p(x: Int) {}\n",
        )]);
        let out = descriptors_to_json(&tests);
        assert!(out.contains("\"ok\":true"), "{out}");
        assert!(out.contains("\"id\":\"p()\""), "{out}");
        assert!(out.contains("\"caseCount\":2"), "{out}");
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
        assert_eq!(
            opts.ids,
            Some(vec!["t()".to_string(), "s/u()".to_string()])
        );
    }

    #[test]
    fn parse_options_tolerates_garbage() {
        let opts = parse_run_options("not json");
        assert!(opts.filter.is_none());
        assert!(opts.ids.is_none());
    }
}
