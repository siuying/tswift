//! Links the native CLI against the system `libsqlite3` used by
//! `src/sqlite_ffi.rs` to back `tswift.db.*` (see
//! `docs/adr/0015-db-host-service-wire.md`).
//!
//! macOS and Linux both ship a system SQLite (`libsqlite3.dylib`/`.so`) as
//! part of the base OS — no vendored/bundled build, no crates.io dependency.
//! Any other target (Windows, wasm, …) is not linked here; `tswift-wasm`
//! never reaches this build script, and `tswift-cli` is not built for those
//! other targets today.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" || target_os == "linux" {
        println!("cargo:rustc-link-lib=dylib=sqlite3");
    }
}
