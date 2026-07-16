//! SwiftUI golden harness (Layers B & C).
//!
//! For every `tests/swiftui-fixtures/<name>.swift`:
//!   * **Layer B** — `tswift swiftui render` must match `<name>.uiir.json`
//!     byte-for-byte.
//!   * **Layer C** — when a `<name>.events.json` exists, `tswift swiftui
//!     dispatch` must match `<name>.patches.json`.
//!
//! Set `UPDATE_GOLDEN=1` to regenerate the `.uiir.json` / `.patches.json`
//! goldens instead of asserting. Adding a fixture is zero-code: drop in a
//! `.swift` (+ optional `.events.json`) and run with `UPDATE_GOLDEN=1` once.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/swiftui-fixtures")
}

fn fixture_filter() -> Option<Vec<String>> {
    std::env::var("TSWIFT_GOLDEN_FILTER").ok().map(|raw| {
        raw.split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

fn matches_filter(path: &Path, filter: Option<&[String]>) -> bool {
    filter.is_none_or(|names| {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| names.iter().any(|name| name == stem))
    })
}

fn swift_fixtures() -> Vec<PathBuf> {
    let filter = fixture_filter();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .expect("swiftui-fixtures dir is readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("swift")
                && matches_filter(p, filter.as_deref())
        })
        .collect();
    paths.sort();
    paths
}

#[test]
fn fixture_filter_matches_exact_stems() {
    let filter = vec!["counter".to_owned()];
    assert!(matches_filter(Path::new("counter.swift"), Some(&filter)));
    assert!(!matches_filter(Path::new("greeting.swift"), Some(&filter)));
    assert!(matches_filter(Path::new("greeting.swift"), None));
}

fn run_cli(args: &[&Path]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_tswift"))
        .args(args)
        .output()
        .expect("failed to spawn tswift");
    assert!(
        output.status.success(),
        "tswift exited with failure on {:?}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).expect("stdout is valid UTF-8")
}

fn update_mode() -> bool {
    std::env::var("UPDATE_GOLDEN").is_ok()
}

fn check_golden(golden: &Path, actual: &str, label: &str) {
    let actual = actual.trim_end_matches('\n');
    if update_mode() {
        std::fs::write(golden, format!("{actual}\n")).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(golden)
        .unwrap_or_else(|_| {
            panic!(
                "missing golden {} — run with UPDATE_GOLDEN=1",
                golden.display()
            )
        })
        .trim_end_matches('\n')
        .to_string();
    assert_eq!(
        actual,
        expected,
        "{label} golden mismatch for {}",
        golden.display()
    );
}

#[test]
fn uiir_goldens_match() {
    let render: &Path = Path::new("swiftui");
    let render2: &Path = Path::new("render");
    for swift in swift_fixtures() {
        let actual = run_cli(&[render, render2, &swift]);
        let golden = swift.with_extension("uiir.json");
        check_golden(&golden, &actual, "UIIR");
    }
}

#[test]
fn patch_goldens_match() {
    let swiftui: &Path = Path::new("swiftui");
    let dispatch: &Path = Path::new("dispatch");
    for swift in swift_fixtures() {
        let events = swift.with_extension("events.json");
        if !events.exists() {
            continue;
        }
        let actual = run_cli(&[swiftui, dispatch, &swift, &events]);
        let golden = swift.with_extension("patches.json");
        check_golden(&golden, &actual, "patch-stream");
    }
}
