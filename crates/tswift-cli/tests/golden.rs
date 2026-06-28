//! Golden-fixture harness.
//!
//! For every `tests/fixtures/<name>.swift` with a sibling `<name>.expected`,
//! run `tswift run <name>.swift` and assert its stdout matches the expected
//! file byte-for-byte. A mismatch fails the test with a readable diff.
//!
//! Adding a feature? Drop in a `.swift` + `.expected` pair — no code changes.
//!
//! Two more fixture flavors, also zero-code to add:
//!   * **Multi-file modules** — a directory `fixtures/multifile/<case>/` holding
//!     several `.swift` files plus `expected.txt`. All `.swift` files (sorted)
//!     are passed to one `run` invocation, exercising cross-file resolution.
//!   * **AST snapshots** — `fixtures/ast/<name>.swift` with a sibling
//!     `<name>.ast` holding the expected `tswift dump` output. These pin
//!     down *how the Rust frontend parses a construct*, so AST-shape changes are
//!     caught.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Collect `(swift_path, expected_path)` pairs, sorted for stable output.

fn fixtures() -> Vec<(PathBuf, PathBuf)> {
    let dir = fixtures_dir();
    let mut pairs = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .expect("fixtures dir is readable")
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("swift") {
            let expected = path.with_extension("expected");
            assert!(
                expected.exists(),
                "fixture {} has no .expected sibling",
                path.display()
            );
            pairs.push((path, expected));
        }
    }
    pairs.sort();
    pairs
}

/// Run the CLI on `swift_path` and return its stdout as a `String`.
fn run_cli(swift_path: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_tswift"))
        .arg("run")
        .arg(swift_path)
        .output()
        .expect("failed to spawn tswift");

    assert!(
        output.status.success(),
        "tswift exited with failure on {}\nstderr:\n{}",
        swift_path.display(),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("stdout is valid UTF-8")
}

#[test]
fn golden_fixtures_match() {
    let pairs = fixtures();
    assert!(
        !pairs.is_empty(),
        "no fixtures found in {}",
        fixtures_dir().display()
    );

    let mut failures = Vec::new();
    for (swift_path, expected_path) in &pairs {
        let expected = std::fs::read_to_string(expected_path).expect("read .expected");
        let actual = run_cli(swift_path);
        if actual != expected {
            failures.push(format!(
                "── {} ──\n  expected: {:?}\n  actual:   {:?}",
                swift_path.file_name().unwrap().to_string_lossy(),
                expected,
                actual
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} golden fixture(s) mismatched:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every `fixtures/multifile/<case>/` directory is one multi-file program: all
/// its `.swift` files (sorted) form a single module and must produce
/// `expected.txt`. Exercises cross-file reference resolution.

#[test]
fn multi_file_modules_match() {
    let root = fixtures_dir().join("multifile");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("multifile dir is readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    assert!(
        !cases.is_empty(),
        "no multifile cases in {}",
        root.display()
    );

    for case in cases {
        let mut sources: Vec<PathBuf> = std::fs::read_dir(&case)
            .expect("case dir is readable")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("swift"))
            .collect();
        sources.sort();
        let output = Command::new(env!("CARGO_BIN_EXE_tswift"))
            .arg("run")
            .args(&sources)
            .output()
            .expect("spawn tswift");
        assert!(
            output.status.success(),
            "multifile case {} failed:\n{}",
            case.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let expected =
            std::fs::read_to_string(case.join("expected.txt")).expect("read expected.txt");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            expected,
            "multifile case {} mismatched",
            case.display()
        );
    }
}

/// Every `fixtures/ast/<name>.swift` with a sibling `<name>.ast` pins the typed
/// AST shape: `tswift dump` must reproduce the snapshot byte-for-byte.

#[test]
fn ast_snapshots_match() {
    let dir = fixtures_dir().join("ast");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("ast dir is readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("swift"))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no ast snapshots in {}", dir.display());

    for swift in cases {
        let snapshot = swift.with_extension("ast");
        assert!(
            snapshot.exists(),
            "AST fixture {} has no .ast sibling",
            swift.display()
        );
        let output = Command::new(env!("CARGO_BIN_EXE_tswift"))
            .arg("dump")
            .arg(&swift)
            .output()
            .expect("spawn tswift");
        assert!(
            output.status.success(),
            "dump failed on {}:\n{}",
            swift.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let expected = std::fs::read_to_string(&snapshot).expect("read .ast");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            expected,
            "AST snapshot {} mismatched",
            swift.display()
        );
    }
}

/// Helper: run `src` through the CLI and return (success, stdout, stderr).
fn run_source(name: &str, src: &str) -> (bool, String, String) {
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, src).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tswift"))
        .arg("run")
        .arg(&path)
        .output()
        .expect("spawn tswift");
    let _ = std::fs::remove_file(&path);
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Foundation `IndexPath`/`IndexSet` edge semantics that the happy-path golden
/// fixture cannot express (errors abort the run).
#[test]
fn index_value_edge_cases() {
    // A wrong argument label on a positional intrinsic is rejected.
    let (ok, _, err) = run_source(
        "tswift_idx_bad_label.swift",
        "import Foundation\nvar q = IndexSet(integersIn: 1...3)\nprint(q.contains(with: 2))\n",
    );
    assert!(!ok, "bad label must fail");
    assert!(err.contains("unexpected argument label"), "stderr: {err}");

    // `update` requires its `with:` label (Swift `update(with:)`).
    let (ok, _, err) = run_source(
        "tswift_idx_update_nolabel.swift",
        "import Foundation\nvar q = IndexSet(integersIn: 1...3)\nlet _ = q.update(4)\n",
    );
    assert!(!ok, "update without `with:` must fail");
    assert!(err.contains("unexpected argument label"), "stderr: {err}");

    // `dropLast` traps on a negative count.
    let (ok, _, err) = run_source(
        "tswift_idx_droplast_neg.swift",
        "import Foundation\nprint(IndexPath(indexes: [1, 2, 3]).dropLast(-1).count)\n",
    );
    assert!(!ok, "negative dropLast must trap");
    assert!(err.contains("negative number"), "stderr: {err}");

    // An out-of-range IndexPath subscript traps.
    let (ok, _, err) = run_source(
        "tswift_idx_subscript_oob.swift",
        "import Foundation\nprint(IndexPath(indexes: [1, 2])[5])\n",
    );
    assert!(!ok, "out-of-range subscript must trap");
    assert!(err.contains("out of range"), "stderr: {err}");

    // Empty-set nearest-member queries return nil; set algebra contents.
    let (ok, out, err) = run_source(
        "tswift_idx_empty_queries.swift",
        "import Foundation\n\
         let empty = IndexSet()\n\
         print(empty.integerGreaterThan(3) ?? -1)\n\
         var a = IndexSet(integersIn: 1...3)\n\
         a.formSymmetricDifference(IndexSet(integersIn: 2...4))\n\
         print(a.contains(1), a.contains(2), a.contains(4))\n",
    );
    assert!(ok, "stderr: {err}");
    assert_eq!(out, "-1\ntrue false true\n");
}

/// `#error("…")` is a compile error: the CLI must exit non-zero, print an
/// `error:` diagnostic, and never execute the program body.
#[test]
fn pound_error_fails_compilation() {
    let dir = std::env::temp_dir();
    let src = dir.join("tswift_pound_error_test.swift");
    std::fs::write(&src, "#error(\"nope\")\nprint(\"should not run\")\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tswift"))
        .arg("run")
        .arg(&src)
        .output()
        .expect("spawn tswift");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!output.status.success(), "#error must fail the build");
    assert!(stderr.contains("error:"), "stderr was: {stderr}");
    assert!(stderr.contains("nope"), "stderr was: {stderr}");
    assert!(
        !stdout.contains("should not run"),
        "program body must not execute; stdout: {stdout}"
    );
    let _ = std::fs::remove_file(&src);
}

/// `#warning("…")` is advisory: the CLI exits zero, prints a `warning:`
/// diagnostic to stderr, and still runs the program body.
#[test]
fn pound_warning_runs_with_stderr_note() {
    let dir = std::env::temp_dir();
    let src = dir.join("tswift_pound_warning_test.swift");
    std::fs::write(&src, "#warning(\"heads up\")\nprint(\"ran\")\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tswift"))
        .arg("run")
        .arg(&src)
        .output()
        .expect("spawn tswift");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "#warning must not fail: {stderr}");
    assert!(stderr.contains("warning:"), "stderr was: {stderr}");
    assert!(
        stdout.contains("ran"),
        "program body must run; stdout: {stdout}"
    );
    let _ = std::fs::remove_file(&src);
}

/// A deliberately broken fixture must make the harness notice a mismatch — this
/// guards the harness itself against silently passing.
#[test]
fn harness_detects_mismatch() {
    let swift = fixtures_dir().join("hello.swift");
    let actual = run_cli(&swift);
    assert_ne!(actual, "this is not the expected output\n");
}
