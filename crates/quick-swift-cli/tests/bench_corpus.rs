//! Validates the benchmark corpus (`benches/programs/*.swift`) actually runs and
//! produces its `.expected` output through the tree-walker.
//!
//! The benchmark harness (`benches/tree_walker.rs`) measures these programs but
//! does not check their results. This test is the correctness guard: if a
//! language change alters a benchmark program's behaviour, the baseline is no
//! longer measuring what we think — and this test fails loudly.

use std::path::{Path, PathBuf};
use std::process::Command;

fn programs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/programs")
}

fn run_cli(swift_path: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_quick-swift"))
        .arg("run")
        .arg(swift_path)
        .output()
        .expect("failed to spawn quick-swift");
    assert!(
        output.status.success(),
        "quick-swift failed on {}\nstderr:\n{}",
        swift_path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout is valid UTF-8")
}

#[test]
fn bench_corpus_runs_and_matches_expected() {
    let dir = programs_dir();
    let mut pairs: Vec<(PathBuf, PathBuf)> = std::fs::read_dir(&dir)
        .expect("benches/programs is readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("swift"))
        .map(|p| {
            let expected = p.with_extension("expected");
            assert!(
                expected.exists(),
                "bench program {} has no .expected sibling",
                p.display()
            );
            (p, expected)
        })
        .collect();
    pairs.sort();

    assert!(
        !pairs.is_empty(),
        "no bench programs found in {}",
        dir.display()
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
        "{} bench program(s) mismatched:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
