use std::path::{Path, PathBuf};

/// Recursively collect every `.c` file under `dir`.
fn collect_c_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_c_sources(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("c") {
            out.push(path);
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // crates/msf-sys -> repo root -> vendor/msf
    let msf_root = manifest_dir
        .join("../../vendor/msf")
        .canonicalize()
        .expect("vendor/msf not found — run `git submodule update --init`");

    let include = msf_root.join("include");
    let generated = msf_root.join("generated");
    let src = msf_root.join("src");
    let uni_include = src.join("unicode/include");
    let uni_src = src.join("unicode/src");

    assert!(
        include.join("msf.h").exists(),
        "{} missing — the msf submodule is not checked out",
        include.join("msf.h").display()
    );

    // ── 1+2. Compile msf's C sources + stub.c into libMiniSwiftFrontend.a ──
    let mut sources = Vec::new();
    collect_c_sources(&src, &mut sources);
    sources.push(manifest_dir.join("stub.c"));

    let mut build = cc::Build::new();
    build
        .std("c11")
        .include(&include)
        .include(&generated)
        .include(&src)
        .include(&uni_include)
        .include(&uni_src)
        .warnings(false)
        .extra_warnings(false);
    for s in &sources {
        build.file(s);
    }
    build.compile("MiniSwiftFrontend");

    // ── 3. Generate Rust bindings from the single public header. ──────────
    let bindings = bindgen::Builder::default()
        .header(manifest_dir.join("wrapper.h").to_string_lossy())
        .clang_arg(format!("-I{}", include.display()))
        .clang_arg(format!("-I{}", generated.display()))
        // Keep the surface tight: only msf's own API + the types it transitively
        // needs. This also keeps the anonymous ASTNode/TypeInfo unions in scope.
        .allowlist_function("msf_.*")
        .allowlist_function("ast_kind_name")
        .allowlist_function("token_.*")
        .allowlist_function("type_.*")
        .allowlist_type("ASTNode")
        .allowlist_type("ASTNodeKind")
        .allowlist_type("TypeInfo")
        .allowlist_type("Token")
        .allowlist_type("TokenType")
        .allowlist_type("Keyword")
        .allowlist_type("OpKind")
        .allowlist_type("Source")
        .allowlist_type("MSFResult")
        .allowlist_var("OP_.*")
        .allowlist_var("KW_.*")
        .allowlist_var("TOK_.*")
        .allowlist_var("AST_.*")
        .allowlist_var("TY_.*")
        .default_enum_style(bindgen::EnumVariation::ModuleConsts)
        .derive_default(false)
        .layout_tests(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("bindgen failed to generate msf bindings");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("failed to write bindings.rs");

    // ── 4. Rebuild triggers. ──────────────────────────────────────────────
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("wrapper.h").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("stub.c").display()
    );
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed={}", include.display());
    // Export include dirs for downstream crates that may need them.
    println!("cargo:include={}", include.display());
}
