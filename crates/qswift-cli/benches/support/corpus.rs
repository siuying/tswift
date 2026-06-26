//! Shared source-of-truth discovery for the benchmark corpus.
//!
//! Both the benchmark harness (`benches/tree_walker.rs`) and the correctness
//! guard (`tests/bench_corpus.rs`) include this file via `#[path]` so they
//! enumerate exactly the same set of programs. Without this, a fixture could be
//! validated by the test yet silently omitted from the benchmark (or vice
//! versa).
//!
//! Programs live in `benches/programs/`. Each `*.swift` source has an
//! `.expected` sibling holding its canonical stdout.

// Included from two compilation units (a bench and an integration test); each
// uses a different subset of these helpers, so unused-item warnings are
// expected in whichever unit doesn't call a given function.
#![allow(dead_code)]

use std::path::PathBuf;

/// A single corpus program: its short name and source/expected paths.
pub struct Program {
    /// File stem, e.g. `fib_recursion`. Used as the benchmark id.
    pub name: String,
    /// Absolute path to the `.swift` source.
    pub source_path: PathBuf,
}

impl Program {
    /// Path to the `.expected` sibling holding canonical stdout.
    pub fn expected_path(&self) -> PathBuf {
        self.source_path.with_extension("expected")
    }

    /// Read the source, panicking with a clear message on failure.
    pub fn read_source(&self) -> String {
        std::fs::read_to_string(&self.source_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", self.source_path.display()))
    }
}

/// The directory holding the corpus, resolved against the crate manifest.
pub fn programs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/programs")
}

/// Discover every `*.swift` program in the corpus, sorted by path for stable
/// ordering. Panics if the directory is unreadable or empty.
pub fn discover() -> Vec<Program> {
    let dir = programs_dir();
    let mut programs: Vec<Program> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read dir {}: {e}", dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|x| x.to_str()) == Some("swift"))
        .map(|source_path| {
            let name = source_path
                .file_stem()
                .expect("swift file has a stem")
                .to_string_lossy()
                .into_owned();
            Program { name, source_path }
        })
        .collect();
    programs.sort_by(|a, b| a.source_path.cmp(&b.source_path));
    assert!(
        !programs.is_empty(),
        "no bench programs found in {}",
        dir.display()
    );
    programs
}
