// Value-type workload: struct construction, value-semantics copies, and
// mutating methods in a loop. Stresses struct alloc/copy and method dispatch.
struct Point {
    var x: Int
    var y: Int
    mutating func step() { x &+= 1; y &+= 2 }
}
var total = 0
var i = 0
while i < 50000 {
    var p = Point(x: i, y: i)
    p.step()
    total &+= p.x &+ p.y
    i += 1
}
print(total)
