// Initializer delegation: convenience → designated (`self.init`) across a
// class hierarchy (TSPL's Food/RecipeIngredient example), initializer
// inheritance, struct init delegation, and failable delegation.

class Vehicle {
  var wheels: Int
  var name: String
  init(wheels: Int, name: String) {
    self.wheels = wheels
    self.name = name
  }
  convenience init() {
    self.init(wheels: 4, name: "car")
  }
  convenience init(bike: Bool) {
    self.init(wheels: 2, name: "bike")
  }
}
let v = Vehicle()
print(v.wheels, v.name)
let bike = Vehicle(bike: true)
print(bike.wheels, bike.name)

class Sports: Vehicle {
  var topSpeed: Int
  init(topSpeed: Int) {
    self.topSpeed = topSpeed
    super.init(wheels: 4, name: "sports")
  }
  override convenience init() {
    self.init(topSpeed: 300)
  }
}
let s = Sports()
print(s.topSpeed, s.wheels, s.name)

// TSPL Food / RecipeIngredient: inherited convenience initializers.
class Food {
  var name: String
  init(name: String) { self.name = name }
  convenience init() { self.init(name: "[unnamed]") }
}
class RecipeIngredient: Food {
  var quantity: Int
  init(name: String, quantity: Int) {
    self.quantity = quantity
    super.init(name: name)
  }
  override convenience init(name: String) {
    self.init(name: name, quantity: 1)
  }
}
let mystery = Food()
print(mystery.name)
let one = RecipeIngredient(name: "bacon")
print(one.name, one.quantity)
let six = RecipeIngredient(name: "eggs", quantity: 6)
print(six.name, six.quantity)
let anon = RecipeIngredient()
print(anon.name, anon.quantity)

// Struct initializer delegation rebuilds and rebinds self.
struct Size {
  var w: Double
  var h: Double
  init(w: Double, h: Double) { self.w = w; self.h = h }
  init(square: Double) {
    self.init(w: square, h: square)
  }
}
let sq = Size(square: 5)
print(sq.w, sq.h)

// Failable delegation: a failing delegate fails the delegating init.
struct Positive {
  var v: Int
  init?(v: Int) {
    if v < 0 { return nil }
    self.v = v
  }
  init?(doubled from: Int) {
    self.init(v: from)
    self.v = self.v * 2
  }
}
print(Positive(v: 4)?.v ?? -1)
print(Positive(doubled: 5)?.v ?? -1)
print(Positive(doubled: -5)?.v ?? -1)
