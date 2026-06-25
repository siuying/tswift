// expected-no-diagnostics
// Tier 1b — labels, defaults, variadics, inout, nesting/capture, function values,
// Never, @discardableResult, tuple returns.

func add(_ a: Int, _ b: Int) -> Int { a + b }

func greet(name: String, greeting: String = "Hello") -> String {
    "\(greeting), \(name)"
}

func sum(_ numbers: Int...) -> Int {
    var total = 0
    for n in numbers { total += n }
    return total
}

func bump(_ x: inout Int) { x += 1 }

func makeCounter() -> () -> Int {
    var count = 0
    func next() -> Int {
        count += 1
        return count
    }
    return next
}

@discardableResult
func record(_ message: String) -> Int { message.count }

func crash() -> Never { fatalError("unreachable") }

func ordered(_ a: Int, _ b: Int) -> (min: Int, max: Int) {
    a <= b ? (a, b) : (b, a)
}

let summed = add(1, 2)
let hello = greet(name: "world")
let variadic = sum(1, 2, 3, 4)
var counter = 0
bump(&counter)
let next = makeCounter()
let firstTick = next()
record("ignored result is fine")
let bounds = ordered(3, 1)

let _ = (summed, hello, variadic, counter, firstTick, bounds.min, bounds.max)
