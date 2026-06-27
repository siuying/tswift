infix operator ^^
func ^^ (base: Int, exp: Int) -> Int {
    var result = 1
    for _ in 0..<exp { result *= base }
    return result
}
print(2 ^^ 8)
print(3 ^^ 3)
