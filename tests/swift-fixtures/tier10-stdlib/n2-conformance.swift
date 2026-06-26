// expected-no-diagnostics
// Tier 10/N2 — builtin conformance accessors: description + hashValue.

let intDesc = 42.description
let doubleDesc = (1.5).description
let boolDesc = true.description
let stringDesc = "hi".description
let arrayDesc = [1, 2, 3].description
let setDesc = Set([7]).description
let dictDesc = [1: 2].description

let intHash = 42.hashValue
let doubleHash = (1.5).hashValue
let boolHash = true.hashValue
let stringHash = "hi".hashValue

let _ = (intDesc, doubleDesc, boolDesc, stringDesc, arrayDesc, setDesc, dictDesc,
         intHash, doubleHash, boolHash, stringHash)
