// expected-no-diagnostics
// Tier 0 — unicode identifiers (NFC) and every comment form.

/* a block comment */
/* an outer /* and a nested */ block comment */
/// A documentation comment attached to `total`.
let café = 1          // a trailing line comment
let 数値 = 2
let _private = 3
let total = café + 数値 + _private
