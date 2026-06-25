#![cfg(not(feature = "rust-backend"))]

//! Validates the benchmark corpus (`benches/programs/*.swift`) actually runs and
//! produces its `.expected` output through the tree-walker.
//!
//! The benchmark harness (`benches/tree_walker.rs`) measures these programs but
//! does not check their results. This test is the correctness guard: if a
//! language change alters a benchmark program's behaviour, the baseline is no
//! longer measuring what we think — and this test fails loudly.
//!
//! Corpus discovery is shared with the benchmark via `benches/support/corpus.rs`
//! so the two can never enumerate different sets of programs.

use std::path::Path;
use std::process::Command;

#[path = "../benches/support/corpus.rs"]
mod corpus;

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
    let programs = corpus::discover();

    let mut failures = Vec::new();
    for program in &programs {
        let expected_path = program.expected_path();
        assert!(
            expected_path.exists(),
            "bench program {} has no .expected sibling",
            program.source_path.display()
        );
        let expected = std::fs::read_to_string(&expected_path).expect("read .expected");
        let actual = run_cli(&program.source_path);
        if actual != expected {
            failures.push(format!(
                "── {} ──\n  expected: {:?}\n  actual:   {:?}",
                program.name, expected, actual
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
