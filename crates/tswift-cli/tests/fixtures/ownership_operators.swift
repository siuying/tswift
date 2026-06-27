// Ownership operators (consume/copy/borrow) and ownership parameter modifiers
// (borrowing/consuming) are accepted; in the tree-walker the operators are
// transparent (evaluate to the operand value).
func describe(_ x: borrowing String) -> Int { x.count }
func take(_ x: consuming [Int]) -> Int { x.count }

let name = "swift"
let moved = consume name
print(moved)

let nums = [1, 2, 3]
let duped = copy nums
print(duped)

print(describe("hello"))
print(take([10, 20]))

struct Widget {
    var id: Int
    consuming func into() -> Int { id }
}
let w = Widget(id: 42)
print(w.into())

struct Resource {
    var handle: Int
    consuming func release() {
        print("releasing \(handle)")
        discard self
    }
}
let r = Resource(handle: 7)
r.release()

// The words remain usable as ordinary identifiers.
var copy = 1
copy += 4
let borrow = 7
func consume() -> Int { 9 }
print(copy + borrow + consume())

// A trailing contextual keyword on one line must not absorb the next line's
// expression: `copy` here is the variable, not the ownership operator.
let aliased = copy
print(aliased)
