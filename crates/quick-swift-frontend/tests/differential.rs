//! Differential test: the pure-Rust frontend vs the C `msf` oracle.
//!
//! For every snippet, we analyze it with **both** backends and compare their
//! *acceptance* verdict (clean vs diagnosed). This is the validation seam for
//! the new frontend: as long as the C oracle exists, it is ground truth for the
//! Swift the runtime relies on, and the Rust pipeline must agree with it on the
//! surface it supports.
//!
//! Scope today is the **Tier 0 + Tier 1a** subset the Rust pipeline parses and
//! types. Two relationships are asserted:
//!
//! - *Agreement on clean code* — inputs both backends accept.
//! - *Rust is stricter* — inputs the permissive C oracle accepts but the Rust
//!   frontend correctly rejects (e.g. a type-annotation mismatch the oracle
//!   never checks). These document where the new frontend already improves on
//!   the oracle.

use quick_swift_frontend::Analysis;

/// The C oracle's verdict: `true` when it reports no diagnostics.
fn c_accepts(src: &str) -> bool {
    Analysis::analyze(src, "diff.swift")
        .expect("C analyze")
        .is_ok()
}

/// The Rust pipeline's verdict: `true` when it parses and resolves cleanly.
fn rust_accepts(src: &str) -> bool {
    match swift_parser::parse(src) {
        Err(_) => false,
        Ok(mut ast) => swift_sema::resolve(&mut ast).is_empty(),
    }
}

/// Tier 0 + Tier 1a snippets both backends must accept.
const CLEAN: &[&str] = &[
    "let x = 1 + 2 * 3",
    "let s = \"hello\"",
    "let flag = true && false",
    "var ratio: Double = 1.5",
    "let big = 1_000_000",
    "let hex = 0xFF",
    "let bin = 0b1010",
    "let oct = 0o17",
    "let f = 3.14",
    "let pair = (1, 2)",
    "let r = 0 ..< 10",
    "let neg = -5",
    "let cmp = 1 < 2",
    "let pick = 1 < 2 ? 10 : 20",
    "print(\"hi\")",
    "let a = 10\nlet b = a + 5",
];

/// Snippets the permissive C oracle accepts but the Rust frontend rejects: a
/// stored binding whose annotation contradicts its initializer.
const RUST_STRICTER: &[&str] = &[
    "let x: Int = \"oops\"",
    "let y: Bool = 3",
    "let z: Double = true",
];

#[test]
fn backends_agree_on_clean_code() {
    let mut disagreements = Vec::new();
    for &src in CLEAN {
        let (c, r) = (c_accepts(src), rust_accepts(src));
        if !(c && r) {
            disagreements.push(format!(
                "{src:?}: c_accepts={c}, rust_accepts={r} (want both true)"
            ));
        }
    }
    assert!(
        disagreements.is_empty(),
        "clean-code disagreements:\n  {}",
        disagreements.join("\n  ")
    );
}

#[test]
fn rust_frontend_is_stricter_than_the_oracle() {
    for &src in RUST_STRICTER {
        assert!(
            c_accepts(src),
            "expected the permissive oracle to accept {src:?}"
        );
        assert!(
            !rust_accepts(src),
            "expected the Rust frontend to reject {src:?}"
        );
    }
}
