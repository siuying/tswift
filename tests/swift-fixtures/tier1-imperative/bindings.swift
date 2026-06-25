// expected-no-diagnostics
// Tier 1a — let/var, inference, compound assignment, ternary, tuples, casts.

let inferred = 42
var mutable: Double = 1.0
mutable += 0.5
mutable *= 2

let ternary = inferred > 0 ? "positive" : "negative"

let pair = (1, "one")
let (number, label) = pair
let viaIndex = pair.0
let _ignored = pair.1

let widened = Int(Int8(100))
let mixed = number + viaIndex + Int(mutable)

let _ = (ternary, label, widened, mixed, _ignored)
