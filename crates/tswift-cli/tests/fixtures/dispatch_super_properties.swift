// Dynamic dispatch + super chains + overridden properties
class Animal {
  var sound: String { "..." }
  func speak() -> String { "\(name()) says \(sound)" }
  func name() -> String { "animal" }
}
class Dog: Animal {
  override var sound: String { "woof" }
  override func name() -> String { "dog" }
}
class Puppy: Dog {
  override var sound: String { "yip " + super.sound }
}
let animals: [Animal] = [Animal(), Dog(), Puppy()]
for a in animals { print(a.speak()) }
// is/as? through hierarchy
for a in animals {
  if let d = a as? Dog {
    print("dog-ish:", d.sound)
  }
  print(a is Puppy)
}
// protocol existentials with default implementations
protocol Greet {
  func hello() -> String
}
extension Greet {
  func hello() -> String { "default" }
  func wave() -> String { "wave-" + hello() }
}
struct Custom: Greet {
  func hello() -> String { "custom" }
}
struct Plain: Greet {}
let gs: [any Greet] = [Custom(), Plain()]
for g in gs { print(g.hello(), g.wave()) }
