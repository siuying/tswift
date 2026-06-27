// Explicit generic specialization accepts contextual keyword case names and
// optional/protocol-composition type arguments.
enum Maybe<T> {
    case some(T)
    case none
}

struct Box<T> {
    var value: Int
}

protocol A {}
protocol B {}
struct BothAB: A, B {}

let m = Maybe<Int>.some(3)
let o = Box<Int?>(value: 4)
let c = Box<A & B>(value: 5)
print(m)
print(o.value)
print(c.value)
