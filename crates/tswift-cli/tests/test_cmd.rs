//! `tswift test` CLI golden tests (Slice B — `docs/plan/swift-testing-support.md`).
//!
//! Each case spawns the real `tswift` binary; fixtures live under
//! `tests/fixtures/test_cmd/`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test_cmd")
}

fn run_test_cmd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tswift"))
        .arg("test")
        .args(args)
        .output()
        .expect("spawn tswift test")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A file with one passing `@Test` exits 0 and reports it as passed.
#[test]
fn passing_run_exits_zero() {
    let file = fixtures_dir().join("passing.swift");
    let output = run_test_cmd(&[file.to_str().unwrap()]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "expected exit 0:\nstdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("addition()"), "stdout: {out}");
    assert!(out.contains("passed"), "stdout: {out}");
    assert!(out.contains("Test run"), "stdout: {out}");
}

/// A failing `#expect` exits 1 and the console output carries the issue's
/// `file:line` and expression detail.
#[test]
fn failing_run_exits_one_with_issue_detail() {
    let file = fixtures_dir().join("failing.swift");
    let output = run_test_cmd(&[file.to_str().unwrap()]);
    let out = stdout(&output);
    assert!(!output.status.success(), "expected exit 1:\nstdout:\n{out}");
    assert!(out.contains("failing.swift:4"), "stdout: {out}");
    assert!(out.contains("add(1, 1) == 3"), "stdout: {out}");
    assert!(out.contains("1 issue"), "stdout: {out}");
}

/// `--filter` runs only the matching test.
#[test]
fn filter_excludes_non_matching_tests() {
    let file = fixtures_dir().join("filter.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--filter", "mathAdd"]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("mathAdd"), "stdout: {out}");
    assert!(!out.contains("mathSub"), "stdout: {out}");
    assert!(out.contains("1 test"), "stdout: {out}");
}

/// A `Package.swift` project with a `.testTarget` (previously rejected as
/// `UnsupportedTargetKind`) loads and runs, concatenating its `Core`
/// dependency's sources so `add` resolves.
#[test]
fn package_with_test_target_runs() {
    let dir = fixtures_dir().join("package");
    let output = run_test_cmd(&[dir.to_str().unwrap()]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("addition()"), "stdout: {out}");
    assert!(out.contains("passed"), "stdout: {out}");
}

/// A `--target` name that isn't a `.testTarget` names its actual kind in the
/// error instead of silently running nothing.
#[test]
fn package_target_flag_selects_named_test_target() {
    let dir = fixtures_dir().join("package");
    let output = run_test_cmd(&[dir.to_str().unwrap(), "--target", "CoreTests"]);
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
}

/// A syntax error is a compile-time failure: nonzero exit, standard
/// diagnostic rendering, and no "Test run started" line.
#[test]
fn compile_error_is_nonzero_with_diagnostic() {
    let file = fixtures_dir().join("compile_error.swift");
    let output = run_test_cmd(&[file.to_str().unwrap()]);
    assert!(!output.status.success(), "expected a compile failure");
    let combined = format!("{}{}", stdout(&output), stderr(&output));
    assert!(combined.contains("compile_error.swift"), "{combined}");
    assert!(!combined.contains("Test run started"), "{combined}");
    // Same rendering as `tswift run`'s diagnostics: `error:` kind, the
    // offending source line, and a caret pointing at the column—not a bare
    // `file:line:col: msg` string.
    assert!(combined.contains(": error:"), "{combined}");
    assert!(combined.contains('^'), "{combined}");
    assert!(combined.contains("@Test func broken("), "{combined}");
}

/// A package with two `.testTarget`s runs both, prints each unit's own
/// `Test run with N tests …` summary, and one aggregate line clearly
/// labeled `Overall:` — never two ambiguous `Test run with …` lines that
/// read like a duplicate/contradictory total.
#[test]
fn two_test_targets_print_per_unit_and_labeled_overall_summary() {
    let dir = fixtures_dir().join("two_targets");
    let output = run_test_cmd(&[dir.to_str().unwrap()]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("Test target CoreTests:"), "stdout: {out}");
    assert!(out.contains("Test target ExtraTests:"), "stdout: {out}");
    let per_unit_summaries = out.matches("Test run with 1 test passed").count();
    assert_eq!(per_unit_summaries, 2, "stdout: {out}");
    assert!(out.contains("Overall: 2 tests passed"), "stdout: {out}");
}

/// An unrecognized `--flag` is a usage error, not silently ignored.
#[test]
fn unknown_flag_is_a_usage_error() {
    let file = fixtures_dir().join("passing.swift");
    let output = run_test_cmd(&["--bogus", file.to_str().unwrap()]);
    assert!(!output.status.success(), "expected a usage error");
    let err = stderr(&output);
    assert!(err.contains("--bogus"), "{err}");
    assert!(err.contains("usage:"), "{err}");
}

/// `--filter` with no following value is a usage error, not a silent `None`
/// (which would otherwise run every test unfiltered).
#[test]
fn filter_with_no_value_is_a_usage_error() {
    let file = fixtures_dir().join("passing.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--filter"]);
    assert!(!output.status.success(), "expected a usage error");
    let err = stderr(&output);
    assert!(err.contains("--filter"), "{err}");
}

/// `--target` with no following value is a usage error, not a silent `None`.
#[test]
fn target_with_no_value_is_a_usage_error() {
    let file = fixtures_dir().join("passing.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--target"]);
    assert!(!output.status.success(), "expected a usage error");
    let err = stderr(&output);
    assert!(err.contains("--target"), "{err}");
}

/// Zero discovered tests is not an error (documented CLI policy, plan §2.5 /
/// R3): exit 0 with a clear "0 tests" message, not a silent false-green.
#[test]
fn zero_tests_exits_zero_with_message() {
    let file = fixtures_dir().join("zero_tests.swift");
    let output = run_test_cmd(&[file.to_str().unwrap()]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("0 tests"), "stdout: {out}");
}
