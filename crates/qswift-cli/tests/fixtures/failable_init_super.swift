// A failing superclass init? must propagate: the subclass initializer that
// calls super.init() also fails, yielding nil rather than a half-built object.

class Base {
    var v: Int
    init?(_ v: Int) {
        if v < 0 { return nil }
        self.v = v
    }
}

class Derived: Base {
    var tag: String
    init?(_ v: Int) {
        self.tag = "d"
        super.init(v)
    }
}

print(Derived(-5) == nil ? "nil" : "instance")
print(Derived(5) == nil ? "nil" : "instance")
if let d = Derived(7) {
    print(d.v)
    print(d.tag)
}
