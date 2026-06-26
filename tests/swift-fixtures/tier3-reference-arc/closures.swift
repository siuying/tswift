// expected-no-diagnostics
// Tier 3a — trailing closures, shorthand args, capture, @escaping, capture
// lists, @autoclosure.

let numbers = [1, 2, 3, 4, 5]
let doubled = numbers.map { $0 * 2 }
let evens = numbers.filter { $0 % 2 == 0 }
let total = numbers.reduce(0) { $0 + $1 }

func apply(_ x: Int, _ transform: (Int) -> Int) -> Int { transform(x) }
let viaTrailing = apply(10) { value in value + 1 }

func makeAdder(_ amount: Int) -> (Int) -> Int {
    return { $0 + amount }
}
let add5 = makeAdder(5)

func defer_(_ action: @escaping () -> Void) -> () -> Void { action }

class Box {
    var value = 0
    func makeIncrementer() -> () -> Void {
        return { [weak self] in self?.value += 1 }
    }
}

func evaluate(_ condition: @autoclosure () -> Bool) -> Bool { condition() }

let box = Box()
let increment = box.makeIncrementer()
increment()
let stored = defer_ { print("ran") }

let _ = (doubled, evens, total, viaTrailing, add5(1), box.value, evaluate(1 < 2), stored)