// Tier 1 — reassigning a `let` constant must be rejected.
let limit = 10
limit = 20 // expected-error{{cannot assign}}
