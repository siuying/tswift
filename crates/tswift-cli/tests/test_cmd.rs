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
    let per_unit_summaries = out.matches("Test run with 1 test, 1 passed").count();
    assert_eq!(per_unit_summaries, 2, "stdout: {out}");
    assert!(out.contains("Overall: 2 tests, 2 passed"), "stdout: {out}");
}

/// A `@Test(arguments:)` runs one case per element, each labelled with its
/// argument value in the console output.
#[test]
fn parameterized_reports_one_case_per_argument() {
    let file = fixtures_dir().join("parameterized.swift");
    let output = run_test_cmd(&[file.to_str().unwrap()]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("divisible(x:) - 4"), "stdout: {out}");
    assert!(out.contains("divisible(x:) - 8"), "stdout: {out}");
    assert!(out.contains("divisible(x:) - 12"), "stdout: {out}");
    assert!(out.contains("3 tests"), "stdout: {out}");
}

/// A `.disabled("reason")` test is skipped (reason shown) and does not fail
/// the run: exit 0, a skip line carrying the reason, and a passing summary
/// that notes the skip count.
#[test]
fn disabled_test_skips_with_reason_and_exits_zero() {
    let file = fixtures_dir().join("skipped.swift");
    let output = run_test_cmd(&[file.to_str().unwrap()]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "a skip is not a failure:\nstdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("skipMe() skipped"), "stdout: {out}");
    assert!(out.contains("under maintenance"), "stdout: {out}");
    assert!(out.contains("1 skipped"), "stdout: {out}");
}

/// `#expect(throws:)` closure matchers: a matching type and `Never.self`
/// pass, while a wrong thrown type fails with both the expected and actual
/// type named in the issue detail.
#[test]
fn expect_throws_matchers_pass_and_report_wrong_type() {
    let file = fixtures_dir().join("throws.swift");
    let output = run_test_cmd(&[file.to_str().unwrap()]);
    let out = stdout(&output);
    assert!(
        !output.status.success(),
        "expected a failure:\nstdout:\n{out}"
    );
    assert!(out.contains("catchesExpectedType()"), "stdout: {out}");
    assert!(out.contains("neverThrows()"), "stdout: {out}");
    // The wrong-type case names both the expected and the actual type.
    assert!(out.contains("Boom"), "stdout: {out}");
    assert!(out.contains("Other"), "stdout: {out}");
    assert!(out.contains("1 issue"), "stdout: {out}");
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

/// `--filter tag:<name>` selects only tests carrying that tag.
#[test]
fn tag_filter_selects_only_tagged_tests() {
    let file = fixtures_dir().join("tags.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--filter", "tag:fast"]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("quick()"), "stdout: {out}");
    assert!(!out.contains("lengthy()"), "stdout: {out}");
    assert!(!out.contains("untagged()"), "stdout: {out}");
    assert!(out.contains("Test run with 1 test"), "stdout: {out}");
}

/// `--filter tag:<name>` matching zero tests exits 0 (plan §2.5 policy) but
/// must not do so silently: the summary names the tag filter so a typo'd tag
/// name is diagnosable from console output, not indistinguishable from a
/// directory with no `@Test`s at all.
#[test]
fn tag_filter_matching_nothing_names_the_filter_in_summary() {
    let file = fixtures_dir().join("tags.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--filter", "tag:nope"]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("0 tests"), "stdout: {out}");
    assert!(out.contains("tag:nope"), "stdout: {out}");
}

/// `--list` prints the discovered tests without running them: each id, a case
/// badge for a parameterized test, and a skip suffix for a disabled one.
#[test]
fn list_prints_discovered_tests_without_running() {
    let file = fixtures_dir().join("list.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--list"]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("addition()"), "stdout: {out}");
    assert!(out.contains("adds two numbers"), "stdout: {out}");
    assert!(out.contains("MathSuite/inSuite()"), "stdout: {out}");
    assert!(out.contains("[3 cases]"), "stdout: {out}");
    assert!(out.contains("skipMe()"), "stdout: {out}");
    assert!(out.contains("under maintenance"), "stdout: {out}");
    // Listing never runs: the disabled test's failing #expect must not surface.
    assert!(!out.contains("recorded an issue"), "stdout: {out}");
    assert!(!out.contains("Test run"), "stdout: {out}");
}

/// `--list --json` emits the shared descriptor wire shape.
#[test]
fn list_json_emits_descriptor_wire_shape() {
    let file = fixtures_dir().join("list.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--list", "--json"]);
    let out = stdout(&output);
    assert!(output.status.success(), "stdout:\n{out}");
    assert!(out.contains("\"ok\":true"), "stdout: {out}");
    assert!(out.contains("\"id\":\"addition()\""), "stdout: {out}");
    assert!(out.contains("\"caseCount\":3"), "stdout: {out}");
    assert!(out.contains("\"suitePath\":\"MathSuite\""), "stdout: {out}");
    assert!(out.contains("\"skipped\":true"), "stdout: {out}");
}

/// `--test <id>` runs exactly the named test.
#[test]
fn test_flag_runs_only_named_test() {
    let file = fixtures_dir().join("filter.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--test", "mathAdd()"]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("mathAdd()"), "stdout: {out}");
    assert!(!out.contains("mathSub()"), "stdout: {out}");
    assert!(out.contains("1 test,"), "stdout: {out}");
}

/// `--test` is repeatable: each occurrence adds an id to the selection.
#[test]
fn repeated_test_flags_run_each_named_test() {
    let file = fixtures_dir().join("filter.swift");
    let output = run_test_cmd(&[
        file.to_str().unwrap(),
        "--test",
        "mathAdd()",
        "--test",
        "mathSub()",
    ]);
    let out = stdout(&output);
    assert!(output.status.success(), "stdout:\n{out}");
    assert!(out.contains("mathAdd()"), "stdout: {out}");
    assert!(out.contains("mathSub()"), "stdout: {out}");
    assert!(out.contains("2 tests"), "stdout: {out}");
}

/// `--test` with an unknown id is an error naming the unknown id, not a silent
/// zero-tests success.
#[test]
fn test_flag_unknown_id_is_an_error() {
    let file = fixtures_dir().join("filter.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--test", "nope()"]);
    assert!(!output.status.success(), "expected a failure");
    let combined = format!("{}{}", stdout(&output), stderr(&output));
    assert!(combined.contains("nope()"), "{combined}");
    assert!(combined.contains("unknown test id"), "{combined}");
}

/// `--test` and `--filter` together are a usage error (mutually exclusive).
#[test]
fn test_and_filter_together_is_a_usage_error() {
    let file = fixtures_dir().join("filter.swift");
    let output = run_test_cmd(&[
        file.to_str().unwrap(),
        "--test",
        "mathAdd()",
        "--filter",
        "math",
    ]);
    assert!(!output.status.success(), "expected a usage error");
    let err = stderr(&output);
    assert!(err.contains("mutually exclusive"), "{err}");
}

/// `--list` over a program that fails to compile surfaces the diagnostic the
/// same way `run`'s does (same render path, `error:` + caret) and exits
/// nonzero, never a silent `ok:true`/empty list.
#[test]
fn list_surfaces_compile_error_and_exits_nonzero() {
    let file = fixtures_dir().join("compile_error.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--list"]);
    assert!(!output.status.success(), "expected a compile failure");
    let combined = format!("{}{}", stdout(&output), stderr(&output));
    assert!(combined.contains("compile_error.swift"), "{combined}");
    assert!(combined.contains(": error:"), "{combined}");
    assert!(combined.contains('^'), "{combined}");
    assert!(!combined.contains("Test run"), "{combined}");
}

/// `--list --json` over a program that fails to compile is a structured
/// `{"ok":false,"compileError":…}` document, not a silent `{"ok":true,
/// "tests":[]}`, and still exits nonzero.
#[test]
fn list_json_surfaces_compile_error() {
    let file = fixtures_dir().join("compile_error.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--list", "--json"]);
    assert!(!output.status.success(), "expected a compile failure");
    let out = stdout(&output);
    assert!(out.contains("\"ok\":false"), "stdout: {out}");
    assert!(out.contains("\"compileError\":"), "stdout: {out}");
    assert!(out.contains("\"tests\":[]"), "stdout: {out}");
}

/// `--list` combined with `--test`/`--filter` is a usage error — listing
/// never runs anything, so a selection flag on it would otherwise silently
/// do nothing.
#[test]
fn list_with_test_or_filter_is_a_usage_error() {
    let file = fixtures_dir().join("list.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--list", "--test", "addition()"]);
    assert!(!output.status.success(), "expected a usage error");
    let err = stderr(&output);
    assert!(err.contains("--list"), "{err}");

    let output = run_test_cmd(&[file.to_str().unwrap(), "--list", "--filter", "add"]);
    assert!(!output.status.success(), "expected a usage error");
    let err = stderr(&output);
    assert!(err.contains("--list"), "{err}");
}

/// A parameterized test's descriptor lists each case's own selectable id
/// (`cases`) and the console table prints each one indented under the test,
/// in the same id form `--test` accepts — not just an opaque `[N cases]`
/// count.
#[test]
fn list_shows_per_case_selectable_ids() {
    let file = fixtures_dir().join("list.swift");
    let output = run_test_cmd(&[file.to_str().unwrap(), "--list"]);
    let out = stdout(&output);
    assert!(output.status.success(), "stdout:\n{out}");
    assert!(out.contains("even() - 2"), "stdout: {out}");
    assert!(out.contains("even() - 4"), "stdout: {out}");
    assert!(out.contains("even() - 6"), "stdout: {out}");

    let output = run_test_cmd(&[file.to_str().unwrap(), "--list", "--json"]);
    let out = stdout(&output);
    assert!(output.status.success(), "stdout:\n{out}");
    assert!(
        out.contains("\"cases\":[\"even() - 2\",\"even() - 4\",\"even() - 6\"]"),
        "stdout: {out}"
    );
}

/// `--list --json` over a multi-`.testTarget` package attributes each
/// descriptor to its owning `target`.
#[test]
fn list_json_multi_target_carries_target_attribution() {
    let dir = fixtures_dir().join("two_targets");
    let output = run_test_cmd(&[dir.to_str().unwrap(), "--list", "--json"]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("\"target\":\"CoreTests\""), "stdout: {out}");
    assert!(out.contains("\"target\":\"ExtraTests\""), "stdout: {out}");
}

/// `--test <id>` over a multi-`.testTarget` package selects the id from
/// whichever unit actually has it, running only that unit — it must not fail
/// the whole run just because the *other* unit doesn't have the id.
#[test]
fn test_flag_selects_across_multiple_targets() {
    let dir = fixtures_dir().join("two_targets");
    let output = run_test_cmd(&[dir.to_str().unwrap(), "--test", "addition()"]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("addition()"), "stdout: {out}");
    assert!(!out.contains("subtraction()"), "stdout: {out}");
    // The unit with no selected ids (ExtraTests) is skipped from the run
    // output entirely — no header, no summary line for it.
    assert!(!out.contains("Test target ExtraTests:"), "stdout: {out}");
    assert!(out.contains("Test target CoreTests:"), "stdout: {out}");
}

/// `--test <id1> --test <id2>` naming one id per target runs both units, each
/// with just its own id.
#[test]
fn test_flag_selects_one_id_per_target_runs_both() {
    let dir = fixtures_dir().join("two_targets");
    let output = run_test_cmd(&[
        dir.to_str().unwrap(),
        "--test",
        "addition()",
        "--test",
        "subtraction()",
    ]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "stdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("Test target CoreTests:"), "stdout: {out}");
    assert!(out.contains("Test target ExtraTests:"), "stdout: {out}");
    assert!(out.contains("addition()"), "stdout: {out}");
    assert!(out.contains("subtraction()"), "stdout: {out}");
}

/// `--test <id>` over a multi-`.testTarget` package where the id matches no
/// unit at all is still a hard error naming the unknown id.
#[test]
fn test_flag_unknown_id_across_targets_is_an_error() {
    let dir = fixtures_dir().join("two_targets");
    let output = run_test_cmd(&[dir.to_str().unwrap(), "--test", "nope()"]);
    assert!(!output.status.success(), "expected a failure");
    let combined = format!("{}{}", stdout(&output), stderr(&output));
    assert!(combined.contains("nope()"), "{combined}");
    assert!(combined.contains("unknown test id"), "{combined}");
}

/// A `withKnownIssue` body's failure is reported as a known issue and does not
/// fail the run; the `.bug(…)` reference does not surface (the test passed).
#[test]
fn known_issue_run_exits_zero_and_reports_known() {
    let file = fixtures_dir().join("known_issue.swift");
    let output = run_test_cmd(&[file.to_str().unwrap()]);
    let out = stdout(&output);
    assert!(
        output.status.success(),
        "known issue must not fail the run:\nstdout:\n{out}\nstderr:\n{}",
        stderr(&output)
    );
    assert!(out.contains("known issue"), "stdout: {out}");
    assert!(out.contains("stillBroken() passed"), "stdout: {out}");
    assert!(out.contains("healthy() passed"), "stdout: {out}");
}
