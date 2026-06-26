class Base {}
class Derived: Base { func extra() -> String { return "extra" } }
let items: [Base] = [Base(), Derived()]
for item in items {
    if let d = item as? Derived {
        print(d.extra())
    } else {
        print("base")
    }
}
print(items[1] is Derived)
