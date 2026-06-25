// oracle-gap: the vendored C msf does not lex Swift regex literals.
// Tier 0 — regex literals (Swift 5.7+), a frontend gap to close.

let word = /[A-Za-z]+/
let phone = #/\d{3}-\d{4}/#

func looksLikeWord(_ s: String) -> Bool {
    return s.contains(/\w+/)
}
