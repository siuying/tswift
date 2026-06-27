// `@dynamicMemberLookup`: a type with subscript(dynamicMember:) accepts
// arbitrary member access, resolved through the subscript.
// expected-no-diagnostics

@dynamicMemberLookup
struct Proxy {
    var storage: [String: Int] = [:]
    subscript(dynamicMember key: String) -> Int {
        storage[key] ?? 0
    }
}

let proxy = Proxy()
let _ = proxy.anything
