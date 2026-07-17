// String initializers beyond scalar conversion:
// `String(repeating:count:)` and `String(_:radix:uppercase:)`.

// ---- repeating:count: ------------------------------------------------------
print(String(repeating: "ab", count: 3))   // ababab
print(String(repeating: "-", count: 5))     // -----
print(String(repeating: "x", count: 0))     // (empty line)
print(String(repeating: "x", count: 0).isEmpty) // true

// ---- radix ----------------------------------------------------------------
print(String(255, radix: 16))               // ff
print(String(255, radix: 16, uppercase: true)) // FF
print(String(10, radix: 2))                 // 1010
print(String(-42, radix: 2))                // -101010
print(String(0, radix: 16))                 // 0
print(String(35, radix: 36))                // z

// ---- scalar conversion still works ----------------------------------------
print(String(42))                           // 42
print(String(describing: [1, 2, 3]))        // [1, 2, 3]

// ---- pattern-match operator (~=) ------------------------------------------
let animal = "cat"
print("cat" ~= animal)                       // true
print("dog" ~= animal)                       // false
