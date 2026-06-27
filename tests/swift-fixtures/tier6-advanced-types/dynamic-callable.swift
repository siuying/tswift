// `@dynamicCallable`: a type with dynamicallyCall(withArguments:) accepts call
// syntax on its instances.
// expected-no-diagnostics

@dynamicCallable
struct Sum {
    func dynamicallyCall(withArguments args: [Int]) -> Int {
        args.reduce(0, +)
    }
}

let sum = Sum()
let _ = sum(1, 2, 3)
