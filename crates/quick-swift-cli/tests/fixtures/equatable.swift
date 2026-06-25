struct Point: Equatable {
    let x: Int
    let y: Int
}
let a = Point(x: 1, y: 2)
let b = Point(x: 1, y: 2)
let c = Point(x: 3, y: 4)
print(a == b)
print(a == c)
print(a != c)
enum Color: Equatable { case red, green, blue }
print(Color.red == Color.red)
print(Color.red == Color.blue)
