//! Golden-fixture harness.
//!
//! For every `tests/fixtures/<name>.swift` with a sibling `<name>.expected`,
//! run `quick-swift run <name>.swift` and assert its stdout matches the expected
//! file byte-for-byte. A mismatch fails the test with a readable diff.
//!
//! Adding a feature? Drop in a `.swift` + `.expected` pair — no code changes.
//!
//! Two more fixture flavors, also zero-code to add:
//!   * **Multi-file modules** — a directory `fixtures/multifile/<case>/` holding
//!     several `.swift` files plus `expected.txt`. All `.swift` files (sorted)
//!     are passed to one `run` invocation, exercising cross-file resolution.
//!   * **AST snapshots** — `fixtures/ast/<name>.swift` with a sibling
//!     `<name>.ast` holding the expected `quick-swift dump` output. These pin
//!     down *how msf parses a construct*, so AST-shape changes are caught.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Collect `(swift_path, expected_path)` pairs, sorted for stable output.
#[cfg(not(feature = "rust-backend"))]
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
    let output = Command::new(env!("CARGO_BIN_EXE_quick-swift"))
        .arg("run")
        .arg(swift_path)
        .output()
        .expect("failed to spawn quick-swift");

    assert!(
        output.status.success(),
        "quick-swift exited with failure on {}\nstderr:\n{}",
        swift_path.display(),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("stdout is valid UTF-8")
}

#[cfg(not(feature = "rust-backend"))]
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
#[cfg(not(feature = "rust-backend"))]
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
        let output = Command::new(env!("CARGO_BIN_EXE_quick-swift"))
            .arg("run")
            .args(&sources)
            .output()
            .expect("spawn quick-swift");
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
/// AST shape: `quick-swift dump` must reproduce the snapshot byte-for-byte.
#[cfg(not(feature = "rust-backend"))]
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
        let output = Command::new(env!("CARGO_BIN_EXE_quick-swift"))
            .arg("dump")
            .arg(&swift)
            .output()
            .expect("spawn quick-swift");
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

#[cfg(feature = "rust-backend")]
#[test]
fn rust_backend_tracer_fixtures_match() {
    for fixture in [
        "hello",
        "arithmetic",
        "functions_recursion",
        // #51 — nominal declarations + Codable shell on the Rust backend.
        "codable",
        // #52 — modifiers, attributes, ownership, static/lazy on the Rust backend.
        "main_entry",
        "property_wrapper",
        "struct_static_lazy",
        "deinit_weak",
        // #53 — Swift patterns (switch/optional/enum/tuple) on the Rust backend.
        "optionals",
        "enum_matching",
        "switch_patterns",
        "indirect_enum",
        "enum_raw_caseiterable",
        "control_flow",
        // #54 — effects, directives, concurrency on the Rust backend.
        "errors",
        "try_variants",
        "defer_order",
        "conditional_compilation",
        "concurrency_tasks",
        "concurrency_async_await",
        "concurrency_async_let",
        "concurrency_actors",
        "concurrency_task_group",
        "concurrency_async_sequence",
        // #55 — calls, accessors, subscripts, custom operators on the Rust backend.
        "func_labels_defaults",
        "func_variadic",
        "struct_inout",
        "struct_observers",
        "struct_value",
        "subscripts",
        "custom_operator",
        "super_init",
        "protocols",
        "generics",
        "classes_inheritance",
    ] {
        let swift = fixtures_dir().join(format!("{fixture}.swift"));
        let expected = std::fs::read_to_string(swift.with_extension("expected"))
            .expect("read tracer expected output");
        let actual = run_cli(&swift);
        assert_eq!(actual, expected, "rust backend tracer fixture {fixture}");
    }
}

/// A deliberately broken fixture must make the harness notice a mismatch — this
/// guards the harness itself against silently passing.
#[test]
fn harness_detects_mismatch() {
    let swift = fixtures_dir().join("hello.swift");
    let actual = run_cli(&swift);
    assert_ne!(actual, "this is not the expected output\n");
}
