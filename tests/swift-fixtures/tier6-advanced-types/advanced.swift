// expected-no-diagnostics
// Tier 6 — opaque `some P`, existential `any P`, metatypes, type(of:), Self,
// implicit member expressions.

protocol Shape {
    func area() -> Double
}

struct Circle: Shape {
    let radius: Double
    func area() -> Double { 3.14159 * radius * radius }
}

struct Square: Shape {
    let side: Double
    func area() -> Double { side * side }
}

func makeShape() -> some Shape {
    Circle(radius: 1)
}

func boxedShape(_ useCircle: Bool) -> any Shape {
    useCircle ? Circle(radius: 2) : Square(side: 2)
}

struct Builder {
    var steps: [String] = []
    static func empty() -> Self { Self() }
    func adding(_ step: String) -> Self {
        var copy = self
        copy.steps.append(step)
        return copy
    }
}

enum Theme {
    case light, dark
    static let preferred: Theme = .dark
}

func currentTheme() -> Theme { .light }

let intMeta: Int.Type = Int.self
let runtimeType = type(of: 42)
let opaque = makeShape()
let boxed = boxedShape(true)
let pipeline = Builder().adding("a").adding("b")

let _ = (intMeta, runtimeType, opaque.area(), boxed.area(), pipeline.steps, Theme.preferred, currentTheme())
