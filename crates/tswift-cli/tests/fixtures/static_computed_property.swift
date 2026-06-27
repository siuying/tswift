// `static var` computed (type-level) properties on struct, class, and enum,
// including unqualified references to sibling statics from within a getter.
struct Math {
    static var pi: Double { 3.14159 }
    static let two = 2.0
    static var tau: Double { pi * two }
}
print(Math.pi)
print(Math.tau)

class Config {
    var level: Int
    init(level: Int) { self.level = level }
    static var standard: Config { Config(level: 3) }
}
print(Config.standard.level)

enum Theme {
    case light, dark
    static var current: Theme { .dark }
}
print(Theme.current)

// Read a static computed property through a generic type parameter.
protocol HasDefault { static var defaultValue: Self { get } }
struct Score: HasDefault {
    var v: Int
    static var defaultValue: Score { Score(v: 100) }
}
func makeDefault<T: HasDefault>(_ example: T) -> T { T.defaultValue }
print(makeDefault(Score(v: 1)).v)
