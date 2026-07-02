// Member assignment through subscript elements: value-semantics CoW for
// struct elements, in-place mutation for class elements, deep nesting, and
// force-unwrapped dictionary elements.
struct P { var x = 0 }
var arr = [P(), P()]
var copy = arr
copy[0].x = 99
print(arr[0].x, copy[0].x)

var d = ["a": P()]
var d2 = d
d2["a"]!.x = 7
print(d["a"]!.x, d2["a"]!.x)

class C { var v = 1 }
let objs = [C(), C()]
objs[1].v = 42
print(objs[0].v, objs[1].v)

struct Inner { var n = 0 }
struct Outer { var inner = Inner() }
var grid = [[Outer()]]
grid[0][0].inner.n += 5
print(grid[0][0].inner.n)

// Index side effects run exactly once; class objects mid-chain participate in
// compound writes; a present optional writes through its force-unwrap.
var calls = 0
func idx() -> Int { calls += 1; return 0 }
var arr2 = [P()]
arr2[idx()].x = 9
print(arr2[0].x, calls)

class Holder { var inner = P() }
struct Box2 { var c = Holder() }
var boxes = [Box2()]
boxes[0].c.inner.x += 5
print(boxes[0].c.inner.x)

struct Q { var maybe: P? = P() }
var qs = [Q()]
qs[0].maybe!.x = 3
print(qs[0].maybe!.x)
