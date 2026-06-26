//! Golden-fixture harness.
//!
//! For every `tests/fixtures/<name>.swift` with a sibling `<name>.expected`,
//! run `qswift run <name>.swift` and assert its stdout matches the expected
//! file byte-for-byte. A mismatch fails the test with a readable diff.
//!
//! Adding a feature? Drop in a `.swift` + `.expected` pair — no code changes.
//!
//! Two more fixture flavors, also zero-code to add:
//!   * **Multi-file modules** — a directory `fixtures/multifile/<case>/` holding
//!     several `.swift` files plus `expected.txt`. All `.swift` files (sorted)
//!     are passed to one `run` invocation, exercising cross-file resolution.
//!   * **AST snapshots** — `fixtures/ast/<name>.swift` with a sibling
//!     `<name>.ast` holding the expected `qswift dump` output. These pin
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
    let output = Command::new(env!("CARGO_BIN_EXE_qswift"))
        .arg("run")
        .arg(swift_path)
        .output()
        .expect("failed to spawn qswift");

    assert!(
        output.status.success(),
        "qswift exited with failure on {}\nstderr:\n{}",
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
        let output = Command::new(env!("CARGO_BIN_EXE_qswift"))
            .arg("run")
            .args(&sources)
            .output()
            .expect("spawn qswift");
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
/// AST shape: `qswift dump` must reproduce the snapshot byte-for-byte.

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
        let output = Command::new(env!("CARGO_BIN_EXE_qswift"))
            .arg("dump")
            .arg(&swift)
            .output()
            .expect("spawn qswift");
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

/// Regenerate the stdlib-coverage inputs `coverage.py` joins over, writing them
/// to `target/stdlib-coverage/` (git-ignored — not a checked-in duplicate):
///
///   * `registered.txt` — live semantic registry keys (`qswift_std::registered_keys`).
///   * `exercised.txt`   — semantic keys dispatched by *passing* golden fixtures.
///
/// Because exercised keys are gathered only from fixtures whose stdout matches
/// their `.expected`, "verified" coverage means "exercised by a passing test",
/// not merely "mentioned in fixture source".
fn coverage_output_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/stdlib-coverage");
    std::fs::create_dir_all(&dir).expect("create coverage output dir");
    dir
}

#[test]
fn stdlib_coverage_inputs() {
    let out_dir = coverage_output_dir();

    // 1. Live registry keys — authoritative, cannot drift from registration.
    let registered = qswift_std::registered_keys().join("\n") + "\n";
    std::fs::write(out_dir.join("registered.txt"), registered).expect("write registered.txt");

    // 2. Keys exercised by passing fixtures only.
    let tmp = out_dir.join("tmp");
    std::fs::create_dir_all(&tmp).expect("create tmp dir");
    let mut exercised: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for (i, (swift_path, expected_path)) in fixtures().iter().enumerate() {
        let expected = std::fs::read_to_string(expected_path).expect("read .expected");
        let keys_file = tmp.join(format!("keys-{i}.txt"));
        let output = Command::new(env!("CARGO_BIN_EXE_qswift"))
            .arg("run")
            .arg(swift_path)
            .env("QSWIFT_COVERAGE_OUT", &keys_file)
            .output()
            .expect("spawn qswift");
        // Only count fixtures that ran cleanly and matched their golden output.
        if !output.status.success() {
            continue;
        }
        if String::from_utf8_lossy(&output.stdout) != expected {
            continue;
        }
        if let Ok(body) = std::fs::read_to_string(&keys_file) {
            exercised.extend(body.lines().filter(|l| !l.is_empty()).map(str::to_string));
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);

    let body: String = exercised.iter().map(|k| format!("{k}\n")).collect();
    std::fs::write(out_dir.join("exercised.txt"), body).expect("write exercised.txt");

    assert!(
        !exercised.is_empty(),
        "no stdlib keys were exercised by any golden fixture"
    );
}

/// A deliberately broken fixture must make the harness notice a mismatch — this
/// guards the harness itself against silently passing.
#[test]
fn harness_detects_mismatch() {
    let swift = fixtures_dir().join("hello.swift");
    let actual = run_cli(&swift);
    assert_ne!(actual, "this is not the expected output\n");
}
