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

/// `Data` subscript and byte-validation edge cases that abort the run.
#[test]
fn data_edge_cases() {
    // Subscript get out of bounds traps.
    let (ok, _, err) = run_source(
        "tswift_data_oob_get.swift",
        "import Foundation\nprint(Data([1, 2, 3])[5])\n",
    );
    assert!(!ok, "OOB Data get must trap");
    assert!(err.contains("out of range"), "stderr: {err}");

    // Subscript set out of bounds traps.
    let (ok, _, err) = run_source(
        "tswift_data_oob_set.swift",
        "import Foundation\nvar d = Data([1, 2, 3])\nd[9] = 42\n",
    );
    assert!(!ok, "OOB Data set must trap");
    assert!(err.contains("out of range"), "stderr: {err}");

    // Assigning an out-of-range Int (999) to a Data subscript traps.
    let (ok, _, err) = run_source(
        "tswift_data_bad_byte_int.swift",
        "import Foundation\nvar d = Data([1, 2, 3])\nd[0] = 999\n",
    );
    assert!(!ok, "out-of-range byte must trap");
    assert!(
        err.contains("0...255") || err.contains("out of range") || err.contains("valid range"),
        "stderr: {err}",
    );

    // Assigning a Bool to a Data subscript traps with a type message.
    let (ok, _, err) = run_source(
        "tswift_data_bad_byte_bool.swift",
        "import Foundation\nvar d = Data([1, 2, 3])\nd[0] = true\n",
    );
    assert!(!ok, "Bool assigned to Data subscript must trap");
    assert!(
        err.contains("UInt8") || err.contains("Bool") || err.contains("byte"),
        "stderr: {err}",
    );
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

/// Trap-path tests for new stdlib collection types.
/// These exit with a non-zero status, so they cannot use the golden-fixture
/// harness (which asserts success). Inline `run_source` is used instead.
#[test]
fn small_collection_traps() {
    // --- ClosedRange: lowerBound > upperBound traps ---
    let (ok, _, err) = run_source(
        "tswift_closedrange_inverted.swift",
        "let _ = 5...1\nprint(\"no trap\")\n",
    );
    assert!(!ok, "inverted ClosedRange must trap");
    assert!(
        err.contains("upperBound < lowerBound") || err.contains("ClosedRange(5...1)"),
        "expected bound-order message, got: {err}"
    );

    // --- CollectionOfOne: subscript[0] succeeds ---
    let (ok, out, _) = run_source(
        "tswift_coo_subscript_ok.swift",
        "let c = CollectionOfOne(99)\nprint(c[0])\n",
    );
    assert!(ok, "CollectionOfOne[0] must succeed");
    assert_eq!(out.trim(), "99");

    // --- CollectionOfOne: subscript[1] traps ---
    let (ok, _, err) = run_source(
        "tswift_coo_subscript_oob.swift",
        "let c = CollectionOfOne(99)\nprint(c[1])\n",
    );
    assert!(!ok, "CollectionOfOne[1] must trap");
    assert!(
        err.contains("out of range"),
        "expected out-of-range message, got: {err}"
    );

    // --- CollectionOfOne: negative subscript traps (subscript_index returns "negative index") ---
    let (ok, _, err) = run_source(
        "tswift_coo_subscript_neg.swift",
        "let c = CollectionOfOne(99)\nprint(c[-1])\n",
    );
    assert!(!ok, "CollectionOfOne[-1] must trap");
    assert!(
        err.contains("negative index") || err.contains("out of range"),
        "expected index-error message, got: {err}"
    );

    // --- ReversedCollection.distance: out-of-bounds `from` traps ---
    let (ok, _, err) = run_source(
        "tswift_rc_distance_oob.swift",
        "let r = [1, 2, 3].reversed()\nprint(r.distance(from: 5, to: 2))\n",
    );
    assert!(!ok, "ReversedCollection.distance OOB must trap");
    assert!(
        err.contains("out of bounds"),
        "expected out-of-bounds message, got: {err}"
    );

    // --- Set.subscript(endIndex): using endIndex as a subscript traps ---
    let (ok, _, err) = run_source(
        "tswift_set_subscript_endindex.swift",
        "var s: Set<Int> = [42]\nlet ei = s.endIndex\nprint(s[ei])\n",
    );
    assert!(!ok, "Set.subscript(endIndex) must trap");
    assert!(
        err.contains("out of range") || err.contains("endIndex"),
        "expected out-of-range message, got: {err}"
    );

    // --- Set stale-index detection: subscript after remove(at:) traps ---
    let (ok, _, err) = run_source(
        "tswift_set_stale_index.swift",
        "var s: Set<Int> = [1, 2]\n\
         let old = s.startIndex\n\
         s.remove(at: old)\n\
         print(s[old])\n",
    );
    assert!(!ok, "stale Set.Index after mutation must trap");
    assert!(
        err.contains("mutated") || err.contains("invalid"),
        "expected mutation-detection message, got: {err}"
    );

    // --- Set stale-index remove(at:) twice traps on second call ---
    let (ok, _, err) = run_source(
        "tswift_set_stale_remove_at.swift",
        "var s: Set<Int> = [1, 2]\n\
         let old = s.startIndex\n\
         s.remove(at: old)\n\
         s.remove(at: old)\n",
    );
    assert!(!ok, "second remove(at:) with stale Set.Index must trap");
    assert!(
        err.contains("mutated") || err.contains("invalid"),
        "expected mutation-detection message, got: {err}"
    );

    // --- Dictionary.subscript(endIndex): using endIndex as a subscript traps ---
    let (ok, _, err) = run_source(
        "tswift_dict_subscript_endindex.swift",
        "var d: [String: Int] = [\"a\": 1]\nlet ei = d.endIndex\nprint(d[ei])\n",
    );
    assert!(!ok, "Dictionary.subscript(endIndex) must trap");
    assert!(
        err.contains("out of range") || err.contains("endIndex"),
        "expected out-of-range message, got: {err}"
    );

    // --- Dictionary stale-index: subscript after remove(at:) traps ---
    let (ok, _, err) = run_source(
        "tswift_dict_stale_index.swift",
        "var d: [String: Int] = [\"a\": 1, \"b\": 2]\n\
         let old = d.startIndex\n\
         d.remove(at: old)\n\
         print(d[old])\n",
    );
    assert!(!ok, "stale Dictionary.Index after mutation must trap");
    assert!(
        err.contains("mutated") || err.contains("invalid"),
        "expected mutation-detection message, got: {err}"
    );

    // --- Set.index(after:) at endIndex traps ---
    let (ok, _, err) = run_source(
        "tswift_set_index_after_end.swift",
        "var s: Set<Int> = [42]\n\
         let ei = s.endIndex\n\
         let _ = s.index(after: ei)\n",
    );
    assert!(!ok, "Set.index(after: endIndex) must trap");
    assert!(
        err.contains("endIndex") || err.contains("out of range") || err.contains("past"),
        "expected past-endIndex message, got: {err}"
    );

    // --- formIndex(after:) at endIndex traps ---
    let (ok, _, err) = run_source(
        "tswift_set_formindex_after_end.swift",
        "var s: Set<Int> = [42]\n\
         var idx = s.endIndex\n\
         s.formIndex(after: &idx)\n",
    );
    assert!(!ok, "Set.formIndex(after: endIndex) must trap");
    assert!(
        err.contains("endIndex") || err.contains("out of range") || err.contains("past"),
        "expected past-endIndex message, got: {err}"
    );
}

/// Operator trap / type-error cases that exit non-zero.
#[test]
fn operator_traps() {
    // --- Int % 0: division by zero traps ---
    let (ok, _, err) = run_source("tswift_rem_by_zero.swift", "print(5 % 0)\n");
    assert!(!ok, "Int % 0 must trap");
    assert!(
        err.contains("division by zero") || err.contains("zero"),
        "expected division-by-zero message, got: {err}"
    );

    // --- Int %= 0: compound remainder by zero also traps ---
    let (ok, _, err) = run_source(
        "tswift_rem_assign_by_zero.swift",
        "var x = 5\nx %= 0\nprint(x)\n",
    );
    assert!(!ok, "Int %=  0 must trap");
    assert!(
        err.contains("division by zero") || err.contains("zero"),
        "expected division-by-zero message, got: {err}"
    );

    // --- Double % Double: operator does not exist in Swift (SE-0067) ---
    let (ok, _, err) = run_source("tswift_double_rem.swift", "print(5.0 % 2.0)\n");
    assert!(!ok, "Double % Double must be a runtime error");
    assert!(
        err.contains("%") || err.contains("truncatingRemainder") || err.contains("Double"),
        "expected % -not-applicable-to-Double message, got: {err}"
    );

    // --- URLComponents.percentEncodedQuery = "%ZZ": invalid percent-encoding traps ---
    let (ok, _, err) = run_source(
        "tswift_urlcomp_bad_percent.swift",
        "import Foundation\nvar c = URLComponents()\nc.percentEncodedQuery = \"%ZZ\"\n",
    );
    assert!(
        !ok,
        "invalid percent-encoding in percentEncodedQuery must trap"
    );
    assert!(
        err.contains("invalid characters") || err.contains("percentEncodedQuery"),
        "expected invalid-characters message, got: {err}"
    );

    // --- URLComponents.percentEncodedPath = "%GG": invalid encoding traps ---
    let (ok, _, err) = run_source(
        "tswift_urlcomp_bad_path_percent.swift",
        "import Foundation\nvar c = URLComponents()\nc.percentEncodedPath = \"%GG\"\n",
    );
    assert!(
        !ok,
        "invalid percent-encoding in percentEncodedPath must trap"
    );
    assert!(
        err.contains("invalid characters") || err.contains("percentEncodedPath"),
        "expected invalid-characters message, got: {err}"
    );

    // --- URLComponents.percentEncodedPath = "/a b": unescaped space (not in
    //     urlPathAllowed) must trap, same as Foundation ---
    let (ok, _, err) = run_source(
        "tswift_urlcomp_space_in_path.swift",
        "import Foundation\nvar c = URLComponents()\nc.percentEncodedPath = \"/a b\"\n",
    );
    assert!(!ok, "unescaped space in percentEncodedPath must trap");
    assert!(
        err.contains("invalid characters") || err.contains("percentEncodedPath"),
        "expected invalid-characters message, got: {err}"
    );

    // --- URLComponents.percentEncodedQuery = "q=a b": unescaped space traps ---
    let (ok, _, err) = run_source(
        "tswift_urlcomp_space_in_query.swift",
        "import Foundation\nvar c = URLComponents()\nc.percentEncodedQuery = \"q=a b\"\n",
    );
    assert!(!ok, "unescaped space in percentEncodedQuery must trap");
    assert!(
        err.contains("invalid characters") || err.contains("percentEncodedQuery"),
        "expected invalid-characters message, got: {err}"
    );

    // --- URLComponents.percentEncodedHost = "host name": unescaped space traps ---
    let (ok, _, err) = run_source(
        "tswift_urlcomp_space_in_host.swift",
        "import Foundation\nvar c = URLComponents()\nc.percentEncodedHost = \"host name\"\n",
    );
    assert!(!ok, "unescaped space in percentEncodedHost must trap");
    assert!(
        err.contains("invalid characters") || err.contains("percentEncodedHost"),
        "expected invalid-characters message, got: {err}"
    );

    // --- PropertyListEncoder.xml must NOT be a valid expression ---
    // Real Foundation places xml/binary on PropertyListSerialization.PropertyListFormat,
    // not on PropertyListEncoder itself. Accessing PropertyListEncoder.xml must error.
    let (ok, _, err) = run_source(
        "tswift_plist_encoder_xml_static.swift",
        "import Foundation\nlet _ = PropertyListEncoder.xml\n",
    );
    assert!(
        !ok,
        "PropertyListEncoder.xml must not be a valid member access"
    );
    assert!(
        err.contains("unknown") || err.contains("xml") || err.contains("PropertyListEncoder"),
        "expected 'unknown member' error, got: {err}"
    );

    // --- removeFirst(Bool): non-Int arg must be a type error, not silent no-arg ---
    let (ok, out, err) = run_source(
        "tswift_remove_first_bool.swift",
        "var a = [1, 2, 3]\na.removeFirst(true)\nprint(a)\n",
    );
    assert!(
        !ok,
        "removeFirst(Bool) must be a type error, not silently remove one element"
    );
    assert!(
        err.contains("Int") || err.contains("type") || err.contains("removeFirst"),
        "expected type-mismatch message, got: {err}"
    );
    assert!(
        !out.contains("[2, 3]"),
        "removeFirst(Bool) must not silently remove the first element (got: {out})"
    );

    // --- removeLast(Bool): non-Int arg must be a type error ---
    let (ok, out, err) = run_source(
        "tswift_remove_last_bool.swift",
        "var a = [1, 2, 3]\na.removeLast(false)\nprint(a)\n",
    );
    assert!(
        !ok,
        "removeLast(Bool) must be a type error, not silently remove one element"
    );
    assert!(
        err.contains("Int") || err.contains("type") || err.contains("removeLast"),
        "expected type-mismatch message, got: {err}"
    );
    assert!(
        !out.contains("[1, 2]"),
        "removeLast(Bool) must not silently remove the last element (got: {out})"
    );

    // --- removeFirst("x"): String arg also errors ---
    let (ok, _, err) = run_source(
        "tswift_remove_first_str.swift",
        "var a = [1, 2, 3]\na.removeFirst(\"x\")\nprint(a)\n",
    );
    assert!(!ok, "removeFirst(String) must be a type error");
    assert!(
        err.contains("Int") || err.contains("type") || err.contains("removeFirst"),
        "expected type-mismatch message, got: {err}"
    );

    // --- removeLast(3.0): Double arg also errors ---
    let (ok, _, err) = run_source(
        "tswift_remove_last_double.swift",
        "var a = [1, 2, 3]\na.removeLast(3.0)\nprint(a)\n",
    );
    assert!(!ok, "removeLast(Double) must be a type error");
    assert!(
        err.contains("Int") || err.contains("type") || err.contains("removeLast"),
        "expected type-mismatch message, got: {err}"
    );

    // --- JSONDecoder.iso8601 must NOT be a valid 2-level member access ---
    // Real Foundation exposes date strategies as `JSONDecoder.DateDecodingStrategy.iso8601`
    // (accessed via `.iso8601` shorthand). The two-level form `JSONDecoder.iso8601` does
    // not exist. The runtime previously registered it as a static integer — now removed.
    let (ok, _, err) = run_source(
        "tswift_json_decoder_iso8601_static.swift",
        "import Foundation\nvar dec = JSONDecoder()\ndec.dateDecodingStrategy = JSONDecoder.iso8601\n",
    );
    assert!(
        !ok,
        "JSONDecoder.iso8601 (2-level) must not be a valid member access"
    );
    assert!(
        err.contains("unknown") || err.contains("iso8601") || err.contains("JSONDecoder"),
        "expected 'unknown member' error, got: {err}"
    );

    // --- JSONEncoder.secondsSince1970 must NOT be a valid 2-level member access ---
    let (ok, _, err) = run_source(
        "tswift_json_encoder_seconds_static.swift",
        "import Foundation\nvar enc = JSONEncoder()\nenc.dateEncodingStrategy = JSONEncoder.secondsSince1970\n",
    );
    assert!(
        !ok,
        "JSONEncoder.secondsSince1970 (2-level) must not be a valid member access"
    );
    assert!(
        err.contains("unknown") || err.contains("secondsSince1970") || err.contains("JSONEncoder"),
        "expected 'unknown member' error, got: {err}"
    );
}
