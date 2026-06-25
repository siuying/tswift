// rust-gap: advanced Tier 0-10 spec syntax not yet modelled by the pure-Rust frontend (tracked in #37)
// Tier 5 — assigning to a `let` inside a catch block must be rejected.
enum Failure: Error { case bad }

func handle() -> Int {
    let fallback = 0
    do {
        throw Failure.bad
    } catch {
        fallback = 1 // expected-error{{cannot assign}}
    }
    return fallback
}