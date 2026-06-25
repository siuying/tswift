// Tier 2 — extensions may not introduce stored properties.
struct Account {
    var balance: Double
}

extension Account {
    var pending: Double = 0 // expected-error{{extensions must not contain stored properties}}
}
