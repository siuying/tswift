import Foundation

let d = UserDefaults.standard

// Typed set/get round-trips.
d.set(true, forKey: "isEnabled")
d.set(42, forKey: "count")
d.set(3.5, forKey: "ratio")
d.set("hello", forKey: "name")
d.set(["a", "b", "c"], forKey: "tags")
d.set(Data("bytes".utf8), forKey: "payload")
d.set(["theme": "dark"], forKey: "settings")

print(d.bool(forKey: "isEnabled"))
print(d.integer(forKey: "count"))
print(d.double(forKey: "ratio"))
print(d.string(forKey: "name")!)
print(d.stringArray(forKey: "tags")!)
print(d.data(forKey: "payload") == Data("bytes".utf8))
print(d.dictionary(forKey: "settings") != nil)

// Missing key defaults.
print(d.bool(forKey: "missing"))
print(d.integer(forKey: "missing"))
print(d.double(forKey: "missing"))
print(d.string(forKey: "missing") == nil)
print(d.stringArray(forKey: "missing") == nil)
print(d.object(forKey: "missing") == nil)

// Coercion: bool(forKey:) on a stored Int.
d.set(0, forKey: "zero")
d.set(5, forKey: "five")
print(d.bool(forKey: "zero"))
print(d.bool(forKey: "five"))

// removeObject(forKey:) and set(nil, forKey:).
d.removeObject(forKey: "isEnabled")
print(d.object(forKey: "isEnabled") == nil)
d.set("temp", forKey: "temp")
d.set(nil, forKey: "temp")
print(d.string(forKey: "temp") == nil)

// `.standard` is a singleton (reference identity).
let d2 = UserDefaults.standard
d2.set(99, forKey: "count")
print(d.integer(forKey: "count"))

// Registered defaults are a fallback: they are visible without persisting or
// overriding an explicit value.
d.register(defaults: ["launches": 7])
print(d.integer(forKey: "launches"))
