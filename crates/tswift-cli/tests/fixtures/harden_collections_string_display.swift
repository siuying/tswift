// Review fix for slice 23 — Collection Display: string escaping.
// Ground-truthed against Swift 6.3.2.
// Verifies that strings inside collections use debugDescription escaping:
// embedded quote → \", backslash → \\, \n \t \r \0, control chars → \u{XX}.

// ── 1. Embedded double-quote ────────────────────────────────────────────────
print(["a\"b"])           // ["a\"b"]

// ── 2. Embedded backslash ────────────────────────────────────────────────────
print(["a\\b"])           // ["a\\b"]

// ── 3. Embedded newline ──────────────────────────────────────────────────────
print(["a\nb"])           // ["a\nb"]

// ── 4. Embedded tab ─────────────────────────────────────────────────────────
print(["a\tb"])           // ["a\tb"]

// ── 5. Embedded carriage return ──────────────────────────────────────────────
print(["a\rb"])           // ["a\rb"]

// ── 6. NUL character ─────────────────────────────────────────────────────────
print(["a\0b"])           // ["a\0b"]

// ── 7. Control character U+01 — \u{01} escape ───────────────────────────────
print(["a\u{01}b"])       // ["a\u{01}b"]

// ── 8. Control character U+0B (vertical tab) ────────────────────────────────
print(["a\u{0B}b"])       // ["a\u{0b}b"]

// ── 9. Control character U+0C (form feed) ───────────────────────────────────
print(["a\u{0C}b"])       // ["a\u{0c}b"]

// ── 10. DEL character U+7F ───────────────────────────────────────────────────
print(["a\u{7F}b"])       // ["a\u{7f}b"]

// ── 11. Backslash + quote together ───────────────────────────────────────────
print(["a\\\"b"])         // ["a\\\"b"]

// ── 12. Empty string ─────────────────────────────────────────────────────────
print([""])               // [""]

// ── 13. Unicode (non-ASCII, no escaping needed) ───────────────────────────────
print(["café"])           // ["café"]

// ── 14. Dict with string key containing embedded quote ────────────────────────
print(["k\"ey": 1])       // ["k\"ey": 1]

// ── 15. Dict with string value containing backslash ───────────────────────────
print(["k": "v\\w"])      // ["k": "v\\w"]

// ── 16. Nested array of strings ───────────────────────────────────────────────
print([["a", "b\nc"]])    // [["a", "b\nc"]]

// ── 17. Array of bools (no quoting) ──────────────────────────────────────────
print([true, false])      // [true, false]

// ── 18. Mixed printable string with special chars ────────────────────────────
print(["line1\nline2\ttabbed"])  // ["line1\nline2\ttabbed"]
