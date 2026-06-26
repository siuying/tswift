func outer(_ x: Int) -> Int {
    func helper(_ y: Int) -> Int { return y * y }
    return helper(x) + helper(x + 1)
}
print(outer(3))
