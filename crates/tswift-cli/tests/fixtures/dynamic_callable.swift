// `@dynamicCallable`: call syntax on an instance routes through its
// dynamicallyCall method — withArguments for positional, withKeywordArguments
// for labelled arguments.
@dynamicCallable
struct Adder {
    func dynamicallyCall(withArguments args: [Int]) -> Int {
        args.reduce(0, +)
    }
}

let add = Adder()
print(add(1, 2, 3))
print(add())

@dynamicCallable
struct Greeter {
    let prefix: String
    func dynamicallyCall(withKeywordArguments args: [String: String]) -> String {
        var parts: [String] = []
        for key in ["first", "last"] {
            if let v = args[key] { parts.append(v) }
        }
        return "\(prefix) " + parts.joined(separator: " ")
    }
}

let hi = Greeter(prefix: "Hello")
print(hi(first: "Ada", last: "Lovelace"))
