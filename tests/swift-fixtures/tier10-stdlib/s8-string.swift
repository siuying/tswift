// expected-no-diagnostics
// Tier 10a/S8 — String / Character / Substring.

let greeting = "Hello, World"
let c = greeting.count
let e = greeting.isEmpty
let f = greeting.first
let l = greeting.last
let up = greeting.uppercased()
let down = greeting.lowercased()
let hp = greeting.hasPrefix("Hello")
let hs = greeting.hasSuffix("World")
let has = greeting.contains("lo, W")
let pre = greeting.prefix(5)
let suf = greeting.suffix(5)

var s = "swift"
s.append("!")
s += "?"
let cat = "a" + "b"

let parts = "a,b,c".split(separator: ",")
let joined = parts.map { $0.uppercased() }.joined(separator: "-")
let rev = String("abc".reversed())

let emoji = "🇺🇸".count
let combining = "e\u{301}".count

let _ = (c, e, f, l, up, down, hp, hs, has, pre, suf, s, cat, parts, joined, rev, emoji, combining)
