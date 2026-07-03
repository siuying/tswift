// Free-function operators: ==, !=, <, <=, >, >=, %, %=, ~=, ===, !==.
// Each is exercised in the infix form most common in Swift; the registry
// entries (registered_free_fn) ensure they appear in registered_keys().

// ---- Equality / inequality (==, !=) ----------------------------------------
// Int
print(7 == 7, 7 != 8)
// Double
print(1.5 == 1.5, 1.5 != 2.0)
// String
print("hello" == "hello", "a" != "b")
// Bool
print(true == true, false != true)

// ---- Ordering (<, <=, >, >=) -----------------------------------------------
// Int
print(3 < 5, 5 <= 5, 5 > 3, 5 >= 5)
// Double
print(1.0 < 2.0, 2.0 <= 2.0, 3.0 > 2.0, 3.0 >= 3.0)
// String (lexicographic)
print("a" < "b", "b" <= "b", "b" > "a", "b" >= "b")

// ---- Remainder / modulo (%, %=) --------------------------------------------
// Int remainder — sign follows the dividend (Swift / Rust default)
print(7 % 3, -7 % 3)
var rem = 10
rem %= 3
print(rem)
// Note: Double % Double does not exist in Swift (removed in Swift 3 / SE-0067).
// Use truncatingRemainder(dividingBy:). Trap cases are in golden.rs operator_traps.

// ---- Pattern-match (~=) ----------------------------------------------------
// ClosedRange ~= Int (containment)
print((1...5) ~= 3, (1...5) ~= 6)
// Half-open range: upper bound is excluded
print((1..<5) ~= 4, (1..<5) ~= 5)
// Equality fallback: T ~= T where T: Equatable
print(42 ~= 42, 42 ~= 41)

// ---- Class identity (===, !==) ---------------------------------------------
class Box {
    var v: Int
    init(_ n: Int) { v = n }
}
let a = Box(1)
let b = Box(1)
let c = a
// c aliases a (same object); b is a distinct allocation.
print(a === c, a !== b, a === b)
