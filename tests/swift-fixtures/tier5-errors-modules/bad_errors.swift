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