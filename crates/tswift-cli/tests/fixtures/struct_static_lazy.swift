struct Circle {
    static let pi = 3
    var radius: Int
    lazy var area: Int = Circle.pi * radius * radius
    func circumference() -> Int { return 2 * Circle.pi * radius }
}
print(Circle.pi)
var c = Circle(radius: 5)
print(c.area)
print(c.circumference())
