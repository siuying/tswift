// A user type shadowing the builtin `Bool` name wins over builtin static
// dispatch: `Bool.random()` resolves to the user method, not the RNG.
struct Bool {
    static func random() -> Int { return 7 }
}
print(Bool.random())
