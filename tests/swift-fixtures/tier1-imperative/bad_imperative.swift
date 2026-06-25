// rust-gap: advanced Tier 0-10 spec syntax not yet modelled by the pure-Rust frontend (tracked in #37)
// Tier 1 — reassigning a `let` constant must be rejected.
let limit = 10
limit = 20 // expected-error{{cannot assign}}