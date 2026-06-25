// Tier 0 — a lexical error: the string literal is never closed.
let greeting = "hello, world   // expected-error{{unterminated}}
