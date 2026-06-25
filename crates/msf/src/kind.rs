//! A real Rust enum over msf's `ASTNodeKind` constants.
//!
//! The enum, its `from_raw` mapping, and the `name` helper are **generated at
//! build time** from msf's `generated/ast_kinds.h` (see `build.rs`), so every
//! kind msf knows about is a named variant — no hand-maintained list to drift,
//! no `Other(N)` archaeology. [`NodeKind::Other`] remains only as a
//! forward-compatible catch-all for a kind newer than the generated table.
//!
//! To inspect the generated source: `target/<profile>/build/msf-*/out/node_kind.rs`.

include!(concat!(env!("OUT_DIR"), "/node_kind.rs"));
