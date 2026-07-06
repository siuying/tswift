// `take()` on an optional STORED property enters Optional dispatch by
// recovering the property's declared type from the struct declaration.
struct Box { var x: Int? }
var b = Box(x: 5)
print(b.x.take())
print(b.x)
