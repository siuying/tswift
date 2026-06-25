// rust-gap: advanced Tier 0-10 spec syntax not yet modelled by the pure-Rust frontend (tracked in #37)
// Tier 2 — extensions may not introduce stored properties.
struct Account {
    var balance: Double
}

extension Account {
    var pending: Double = 0 // expected-error{{extensions must not contain stored properties}}
}