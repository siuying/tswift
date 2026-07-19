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
fn issue_record_soft_fails_the_test() {
    let src = "@Test func t() { Issue.record(\"boom\") }\n";
    let report = run(src);
    assert_eq!(report.failed(), 1, "tests: {:?}", report.tests);
    assert!(report.tests[0].issues[0].message.contains("boom"));
}

#[test]
fn issue_record_continues_the_body() {
    // A recorded issue is soft: the body runs on, so a later #expect also
    // records, yielding two issues on one failed test.
    let src = "@Test func t() { Issue.record(\"first\")\n#expect(false) }\n";
    let report = run(src);
    assert_eq!(report.failed(), 1);
    assert_eq!(report.tests[0].issues.len(), 2, "issues: {:?}", report.tests[0].issues);
}

#[test]
fn parameterized_expands_one_case_per_argument() {
    let src = "@Test(arguments: [1, 2, 3]) func positive(x: Int) { #expect(x > 0) }\n";
    let report = run(src);
    assert_eq!(report.passed(), 3, "tests: {:?}", report.tests);
    assert_eq!(report.tests.len(), 3);
}

#[test]
fn parameterized_failure_is_isolated_to_its_case() {
    let src = "@Test(arguments: [4, -1, 8]) func positive(x: Int) { #expect(x > 0) }\n";
    let report = run(src);
    assert_eq!(report.passed(), 2, "tests: {:?}", report.tests);
    assert_eq!(report.failed(), 1);
    let failed = report.tests.iter().find(|t| t.status == TestStatus::Failed).unwrap();
    assert!(failed.label().contains("-1"), "label: {}", failed.label());
}

#[test]
fn parameterized_cartesian_product() {
    let src = "@Test(arguments: [1, 2], [10, 20]) func pair(x: Int, y: Int) { #expect(x < y) }\n";
    let report = run(src);
    assert_eq!(report.tests.len(), 4, "cartesian: {:?}", report.tests);
    assert_eq!(report.passed(), 4);
}

#[test]
fn parameterized_zip_pairs_arguments() {
    let src = "@Test(arguments: zip([1, 2], [1, 2])) func eq(a: Int, b: Int) { #expect(a == b) }\n";
    let report = run(src);
    assert_eq!(report.tests.len(), 2, "zip: {:?}", report.tests);
    assert_eq!(report.passed(), 2);
}

#[test]
fn disabled_trait_skips_with_reason() {
    let src = "@Test(.disabled(\"flaky\")) func t() { #expect(false) }\n";
    let report = run(src);
    assert_eq!(report.skipped(), 1);
    assert_eq!(report.failed(), 0);
    assert!(report.is_success(), "a skip is not a failure");
    let t = &report.tests[0];
    assert_eq!(t.status, TestStatus::Skipped);
    assert_eq!(t.skip_reason.as_deref(), Some("flaky"));
}

#[test]
fn enabled_if_false_skips_and_true_runs() {
    let off = run("@Test(.enabled(if: 1 > 2)) func t() { #expect(false) }\n");
    assert_eq!(off.skipped(), 1, "tests: {:?}", off.tests);
    assert_eq!(off.failed(), 0);

    let on = run("@Test(.enabled(if: 2 > 1)) func t() { #expect(true) }\n");
    assert_eq!(on.passed(), 1, "tests: {:?}", on.tests);
    assert_eq!(on.skipped(), 0);
}

#[test]
fn suite_level_disabled_applies_to_all_members() {
    let src = "\
@Suite(.disabled(\"whole suite\"))
struct S {
  @Test func a() { #expect(false) }
  @Test func b() { #expect(false) }
}
";
    let report = run(src);
    assert_eq!(report.skipped(), 2, "tests: {:?}", report.tests);
    assert_eq!(report.failed(), 0);
    assert!(report.tests.iter().all(|t| t.skip_reason.as_deref() == Some("whole suite")));
}

#[test]
fn suite_and_test_display_names_appear_in_label() {
    let src = "\
@Suite(\"Math Suite\") struct M {
  @Test(\"adds two numbers\") func add() { #expect(true) }
}
";
    let report = run(src);
    let label = report.tests[0].label();
    assert!(label.contains("adds two numbers"), "label: {label}");
    assert!(label.contains("Math Suite"), "label: {label}");
}

#[test]
fn nested_suite_types_are_discovered() {
    let src = "\
struct Outer {
  @Test func a() { #expect(true) }
  struct Inner {
    @Test func b() { #expect(true) }
  }
}
";
    let report = run(src);
    assert_eq!(report.passed(), 2, "issues: {:?}", report.tests);
    assert!(
        report.tests.iter().any(|t| t.id == "Outer/Inner/b()"),
        "ids: {:?}",
        report.tests.iter().map(|t| &t.id).collect::<Vec<_>>()
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
