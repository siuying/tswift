class Shape {
    var sides: Int
    init(_ s: Int) { sides = s }
    func describe() -> String { return "shape with \(sides) sides" }
}
class Square: Shape {
    init() { super.init(4) }
    override func describe() -> String { return "square: " + super.describe() }
}
let sq = Square()
print(sq.describe())
print(sq.sides)
