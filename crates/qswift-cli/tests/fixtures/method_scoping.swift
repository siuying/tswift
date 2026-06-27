// Method bodies are isolated from the caller's locals, and Swift name
// resolution applies: local vars/params, then the type's members (which shadow
// module globals), then globals.

// A method must not see the caller's local variable of the same name.
struct Square { var s: Double; func area() -> Double { s * s } }
func compute() -> Double {
    let s = "caller local"   // must not leak into area()
    print(s)
    return Square(s: 4).area()
}
print(compute())

// A property shadows a global of the same name for both reads and writes.
var x = 99
struct Counter {
    var x: Int
    mutating func bump() { x += 1 }
    func read() -> Int { x }
}
var c = Counter(x: 10)
c.bump()
print(c.read())
print(x)

// A mutating method that throws still copies its mutations back to the caller.
struct Boom: Error {}
struct Acc {
    var n: Int
    mutating func addThenThrow() throws { n += 1; throw Boom() }
}
var a = Acc(n: 5)
do { try a.addThenThrow() } catch {}
print(a.n)

// Methods still reach module globals (functions, constants).
func helper() -> Int { 7 }
let factor = 3
struct Calc { var base: Int; func go() -> Int { base * helper() * factor } }
print(Calc(base: 2).go())
