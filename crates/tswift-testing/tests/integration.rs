//! Integration tests driving the runner through real Swift Testing source
//! fixtures (plan §6 slice A "TDD first tests").

use tswift_frontend::SourceFile;
use tswift_testing::{run_tests, RunOptions, TestStatus};

fn run(src: &str) -> tswift_testing::RunReport {
    run_tests(
        &[SourceFile::new("Tests.swift", src)],
        &RunOptions::default(),
    )
}

#[test]
fn passing_test_passes() {
    let report = run("import Testing\n@Test func addition() { #expect(1 + 1 == 2) }\n");
    assert_eq!(report.passed(), 1);
    assert_eq!(report.failed(), 0);
    assert!(report.is_success());
}

#[test]
fn failing_expect_records_issue_with_detail() {
    let src = "\
func add(_ a: Int, _ b: Int) -> Int { a + b }
@Test func addition() { #expect(add(1, 1) == 3) }
";
    let report = run(src);
    assert_eq!(report.failed(), 1);
    assert!(!report.is_success());
    let issue = &report.tests[0].issues[0];
    assert!(
        issue.message.contains("add(1, 1) == 3"),
        "message should carry the expression spelling: {}",
        issue.message
    );
    assert!(
        issue.message.contains("→ 2"),
        "message should carry the operand value: {}",
        issue.message
    );
    assert_eq!(issue.file.as_deref(), Some("Tests.swift"));
    assert_eq!(issue.line, 2);
}

#[test]
fn expect_continues_after_failure() {
    let src = "@Test func t() { #expect(false)\n#expect(false) }\n";
    let report = run(src);
    assert_eq!(report.failed(), 1);
    assert_eq!(report.tests[0].issues.len(), 2, "both #expect run");
}

#[test]
fn require_aborts_body_but_next_test_runs() {
    let src = "\
@Test func first() { #require(false)\n#expect(false) }
@Test func second() { #expect(true) }
";
    let report = run(src);
    // `first` records exactly one issue (the #require) — the following #expect
    // never runs.
    let first = report.tests.iter().find(|t| t.id == "first()").unwrap();
    assert_eq!(first.status, TestStatus::Failed);
    assert_eq!(first.issues.len(), 1);
    // `second` still runs and passes — the abort was test-local.
    let second = report.tests.iter().find(|t| t.id == "second()").unwrap();
    assert_eq!(second.status, TestStatus::Passed);
}

#[test]
fn try_require_unwraps_optional() {
    let src = "\
@Test func unwrap() throws {
  let opt: Int? = 5
  let x = try #require(opt)
  #expect(x == 5)
}
";
    let report = run(src);
    assert_eq!(report.passed(), 1, "issues: {:?}", report.tests[0].issues);
}

#[test]
fn throwing_test_fails() {
    let src = "\
struct Boom: Error {}
@Test func boom() throws { throw Boom() }
";
    let report = run(src);
    assert_eq!(report.failed(), 1);
    assert!(report.tests[0].issues[0]
        .message
        .to_lowercase()
        .contains("error"));
}

#[test]
fn async_test_runs() {
    let src = "\
func value() async -> Int { 42 }
@Test func asyncTest() async { #expect(await value() == 42) }
";
    let report = run(src);
    assert_eq!(report.passed(), 1, "issues: {:?}", report.tests[0].issues);
}

#[test]
fn suite_instance_is_fresh_per_test() {
    // A mutation in one test must not leak into the next: each test gets a new
    // suite instance (Apple's isolation model).
    let src = "\
struct Counter {
  var count = 0
  @Test mutating func first() { count += 1\n#expect(count == 1) }
  @Test mutating func second() { count += 1\n#expect(count == 1) }
}
";
    let report = run(src);
    assert_eq!(
        report.passed(),
        2,
        "fresh instance per test; issues: {:?}",
        report.tests.iter().map(|t| &t.issues).collect::<Vec<_>>()
    );
}

#[test]
fn filter_selects_matching_tests() {
    let src = "@Test func alpha() { #expect(true) }\n@Test func beta() { #expect(true) }\n";
    let report = run_tests(
        &[SourceFile::new("Tests.swift", src)],
        &RunOptions {
            filter: Some("alpha".to_string()),
        },
    );
    assert_eq!(report.tests.len(), 1);
    assert_eq!(report.tests[0].id, "alpha()");
}

#[test]
fn compile_error_yields_no_tests() {
    let report = run("@Test func t() { let x = }\n");
    assert!(report.compile_error.is_some());
    assert!(!report.is_success());
    assert!(report.tests.is_empty());
}
