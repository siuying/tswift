// expected-no-diagnostics
// Tier 10/N2 follow-up — debugDescription + collection/Optional hashValue.

let doubleDebug = (1.5).debugDescription
let stringDebug = "hi".debugDescription
let arrayDebug = ["x", "y"].debugDescription
let setDebug = Set([1]).debugDescription
let dictDebug = [1: "a"].debugDescription

let n: Int? = nil
let nilDebug = n.debugDescription

let arrayHash = [1, 2, 3].hashValue
let setHash = Set([1, 2]).hashValue
let dictHash = [1: "a"].hashValue

let _ = (doubleDebug, stringDebug, arrayDebug, setDebug, dictDebug, nilDebug,
         arrayHash, setHash, dictHash)
