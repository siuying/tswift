enum MathError: Error { case divByZero }
func divide(_ a: Int, _ b: Int) -> Result<Int, MathError> {
    if b == 0 { return .failure(.divByZero) }
    return .success(a / b)
}
let r1 = divide(10, 2)
let r2 = divide(5, 0)
do {
    print(try r1.get())
    print(try r2.get())
} catch MathError.divByZero {
    print("cannot divide by zero")
}
