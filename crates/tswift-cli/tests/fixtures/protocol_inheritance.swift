// Protocol inheritance: a refining protocol carries its parents' requirements,
// conformance satisfies the whole chain, and inherited default implementations
// are visible.

// 1. A refining protocol requires the parent's members too.
protocol Animal { func sound() -> String }
protocol Pet: Animal { var name: String { get } }
struct Dog: Pet {
    let name: String
    func sound() -> String { "Woof" }
}
func describe(_ p: Pet) -> String { "\(p.name) says \(p.sound())" }
let rex = Dog(name: "Rex")
print(describe(rex))

// A `Pet` existential is usable where the inherited `Animal` is expected
// (existential-to-existential upcast through the inheritance edge).
let p: Pet = rex
let a: Animal = p
print(a.sound())

// 2. Inherited default implementations from extensions up the chain.
protocol Greeter { func greet() -> String }
extension Greeter { func greet() -> String { "Hello" } }
protocol FormalGreeter: Greeter { func bow() -> String }
extension FormalGreeter { func bow() -> String { "*bows*" } }
struct Butler: FormalGreeter {}
let b = Butler()
print(b.greet(), b.bow())
// Inherited defaults also dispatch through the protocol existential.
func salute(_ x: FormalGreeter) -> String { "\(x.greet()) \(x.bow())" }
print(salute(b))

// 3. Multiple protocol inheritance.
protocol HasA { func a() -> Int }
protocol HasB { func b() -> Int }
protocol HasAB: HasA, HasB { func c() -> Int }
struct Combo: HasAB {
    func a() -> Int { 1 }
    func b() -> Int { 2 }
    func c() -> Int { 3 }
}
let combo = Combo()
print(combo.a() + combo.b() + combo.c())
func useA(_ x: HasA) -> Int { x.a() }
func useB(_ x: HasB) -> Int { x.b() }
print(useA(combo), useB(combo))

// 4. A three-level inheritance chain; conformance only restates the root.
protocol L1 { func tag() -> String }
protocol L2: L1 {}
protocol L3: L2 {}
struct Leaf: L3 { func tag() -> String { "leaf" } }
func needsL1(_ x: L1) -> String { x.tag() }
print(needsL1(Leaf()))
