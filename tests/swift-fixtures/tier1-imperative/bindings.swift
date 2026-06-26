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

// Tuple-destructuring assignment (swap + multi-element).
var s0 = 0, s1 = 1
(s0, s1) = (s1, s0 + s1)

// Integer literal in a Double context coerces; mixed arithmetic is Double.
let scaled: Double = 5
let half = scaled / 2

let _ = (ternary, label, widened, mixed, _ignored, s0, s1, half)
