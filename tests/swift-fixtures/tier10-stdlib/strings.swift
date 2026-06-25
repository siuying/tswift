// expected-no-diagnostics
// Tier 10a — String, Character, Substring, and common operations.

let greeting = "Hello, world"
let length = greeting.count
let shouted = greeting.uppercased()
let hasWorld = greeting.contains("world")
let firstChar: Character = greeting.first ?? " "
let head: Substring = greeting.prefix(5)
let joined = ["a", "b", "c"].joined(separator: "-")
let interpolated = "length=\(length) first=\(firstChar)"
let reversed = String(greeting.reversed())

let _ = (length, shouted, hasWorld, firstChar, head, joined, interpolated, reversed)
