// Existential `any P` and opaque `some P` types, plus the method-scoping rule
// that lets a property shadow a same-named global / outer local inside a method.
protocol Shape { func area() -> Double }
struct Circle: Shape { var r: Double; func area() -> Double { 3.0 * r * r } }
struct Rect: Shape { var w: Double; var h: Double; func area() -> Double { w * h } }

// Existential array: heterogeneous conformers behind `any Shape`.
let shapes: [any Shape] = [Circle(r: 2), Rect(w: 3, h: 4)]
var total = 0.0
for s in shapes { total += s.area() }
print(total)

// `any P` as a parameter type.
func describe(_ shape: any Shape) -> Double { shape.area() }
print(describe(Circle(r: 1)))

// Opaque return type `some P`.
func defaultShape() -> some Shape { Circle(r: 5) }
print(defaultShape().area())

// A property `s` shadows the global `s` inside the method body.
struct Square { var s: Double; func area() -> Double { s * s } }
let s = Square(s: 6)
print(s.area())
