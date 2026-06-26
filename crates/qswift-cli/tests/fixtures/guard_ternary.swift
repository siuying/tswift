func sign(_ x: Int) -> String {
    guard x != 0 else { return "zero" }
    return x > 0 ? "positive" : "negative"
}
print(sign(5))
print(sign(-3))
print(sign(0))
