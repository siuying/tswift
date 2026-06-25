// expected-no-diagnostics
// Tier 3 — is / as? / as! / as, downcasting, and casting through Any.

class Shape {
    func area() -> Double { 0 }
}

class Circle: Shape {
    let radius: Double
    init(radius: Double) { self.radius = radius }
    override func area() -> Double { 3.14159 * radius * radius }
}

class Square: Shape {
    let side: Double
    init(side: Double) { self.side = side }
    override func area() -> Double { side * side }
}

let shapes: [Shape] = [Circle(radius: 2), Square(side: 3)]

var circleCount = 0
var radii = 0.0
for shape in shapes {
    if shape is Circle { circleCount += 1 }
    if let circle = shape as? Circle { radii += circle.radius }
}

let definitelyCircle = shapes[0] as! Circle
let asBase = definitelyCircle as Shape

let anything: Any = 42
let recovered = anything as? Int

let _ = (circleCount, radii, definitelyCircle.radius, asBase.area(), recovered)
