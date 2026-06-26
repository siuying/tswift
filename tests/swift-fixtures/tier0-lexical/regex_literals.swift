// expected-no-diagnostics
// Tier 0 — regex literals (Swift 5.7+): bare `/.../` and extended `#/.../#`.

let word = /[A-Za-z]+/
let phone = #/\d{3}-\d{4}/#
let alt = /cat|dog/

func looksLikeWord(_ s: String) -> Bool {
    return s.contains(/\w+/)
}

// A `/` after a value is still division, not a regex literal.
let ratio = 22 / 7
let scaled = ratio / 2
