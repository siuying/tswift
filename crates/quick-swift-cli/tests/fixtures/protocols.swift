protocol Shape {
    var area: Int { get }
}
extension Shape {
    func describe() -> String { return "area is \(area)" }
}
struct Square: Shape { let side: Int; var area: Int { side * side } }
struct Rectangle: Shape { let w: Int; let h: Int; var area: Int { w * h } }
let shapes: [any Shape] = [Square(side: 4), Rectangle(w: 2, h: 5)]
for s in shapes {
    print(s.describe())
}
print(shapes.map { $0.area })
