// `@dynamicMemberLookup`: member access falls back to subscript(dynamicMember:).
@dynamicMemberLookup
struct JSONObject {
    var storage: [String: Int] = ["a": 1, "b": 2]
    subscript(dynamicMember key: String) -> Int {
        storage[key] ?? -1
    }
}

let obj = JSONObject()
print(obj.a)
print(obj.b)
print(obj.c)

// Returning a String from the dynamic subscript.
@dynamicMemberLookup
struct Settings {
    subscript(dynamicMember name: String) -> String {
        "setting:\(name)"
    }
}

let s = Settings()
print(s.theme)
print(s.locale)

// A @dynamicMemberLookup type with an ordinary Int subscript: member access
// must use the String-keyed dynamic subscript, not the Int one.
@dynamicMemberLookup
struct Mixed {
    var items = [10, 20, 30]
    subscript(_ i: Int) -> Int { items[i] }
    subscript(dynamicMember key: String) -> String { "member:\(key)" }
}

let m = Mixed()
print(m[1])
print(m.foo)
