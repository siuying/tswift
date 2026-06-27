// `#selector` yields a method's name; `#keyPath` yields the dotted key-path
// string relative to the root type.
class Model {
    @objc var name = "n"
    @objc var nested = Inner()
    @objc func update() {}
    @objc func apply(_ x: Int) {}
}

class Inner {
    @objc var value = 0
}

print(#selector(Model.update))
print(#selector(Model.apply(_:)))
print(#selector(getter: Model.name))
print(#keyPath(Model.name))
print(#keyPath(Model.nested.value))
