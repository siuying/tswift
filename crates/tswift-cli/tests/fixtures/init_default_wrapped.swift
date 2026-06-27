// A custom initializer seeds stored-property defaults before its body runs,
// including @propertyWrapper-backed properties (which are wrapped just like in
// the synthesized memberwise initializer).
@propertyWrapper
struct Clamped {
    private var value: Int
    let limit: Int
    init(wrappedValue: Int, limit: Int = 10) {
        self.limit = limit
        self.value = min(wrappedValue, limit)
    }
    var wrappedValue: Int {
        get { value }
        set { value = min(newValue, limit) }
    }
}

struct Config {
    @Clamped var level = 5
    var name: String
    var enabled = true
    init(name: String) { self.name = name }
}

var c = Config(name: "demo")
print(c.level)
print(c.name)
print(c.enabled)
c.level = 99
print(c.level)
