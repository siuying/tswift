//! Integration test that builds the actual wasm artifact and runs it through
//! Node against the web-sandbox smoke suite.
//!
//! This is the only check that catches wasm-only regressions (e.g. a panic from
//! `SystemTime::now()`, which is unimplemented on wasm32 and aborts to
//! `RuntimeError: unreachable`). Native unit tests in `src/lib.rs` cannot — the
//! host has a working clock.
//!
//! It is part of `cargo test`, but skips gracefully when the JS toolchain is
//! unavailable: install it once with `npm install` in `prototype/web-sandbox`.

use std::path::PathBuf;
use std::process::Command;

fn prototype_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")) // crates/tswift-wasm
        .parent()
        .and_then(|p| p.parent()) // repo root
        .expect("locate repo root")
        .join("prototype")
        .join("web-sandbox")
}

fn tool_available(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn wasm_smoke_runs_compiled_artifact() {
    let dir = prototype_dir();

    if !tool_available("node") {
        eprintln!("skipping wasm smoke test: `node` not found on PATH");
        return;
    }

    let wasm_pack = dir.join("node_modules").join(".bin").join("wasm-pack");
    if !wasm_pack.exists() {
        eprintln!(
            "skipping wasm smoke test: run `npm install` in {} first",
            dir.display()
        );
        return;
    }

    // Build the wasm artifact the smoke test loads.
    let build = Command::new(&wasm_pack)
        .current_dir(&dir)
        .args([
            "build",
            "../../crates/tswift-wasm",
            "--target",
            "web",
            "--out-dir",
            "../../prototype/web-sandbox/src/wasm",
            "--out-name",
            "tswift_wasm",
        ])
        .status()
        .expect("invoke wasm-pack");
    assert!(build.success(), "wasm-pack build failed");

    // Run the real wasm through Node.
    let smoke = Command::new("node")
        .current_dir(&dir)
        .arg("test/wasm-smoke.mjs")
        .status()
        .expect("invoke node");
    assert!(smoke.success(), "wasm smoke test failed (see output above)");
}
