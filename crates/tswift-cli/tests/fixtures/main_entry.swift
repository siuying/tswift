func fib(_ n: Int) -> Int { return n < 2 ? n : fib(n - 1) + fib(n - 2) }
@main
struct Program {
    static func main() {
        for i in 0..<8 {
            print(fib(i))
        }
    }
}
