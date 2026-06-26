// Metatypes (`T.self`) and dynamic type (`type(of:)`).

print(Int.self)
print(String.self)

let x = 42
print(type(of: x))

struct Point { let x: Int; let y: Int }
print(Point.self)
print(type(of: Point(x: 1, y: 2)))

// Metatype identity comparison.
print(Int.self == type(of: x))
print(Int.self == String.self)

// Dynamic type respects the runtime class, not the static type.
class Animal { }
class Dog: Animal { }
let pet: Animal = Dog()
print(type(of: pet))
