import Foundation

// -- percentEncodedPath getter --
let c1 = URLComponents(string: "https://host.com/a%20b/c%20d")!
print(c1.path)
print(c1.percentEncodedPath)

// -- percentEncodedQuery getter --
let c2 = URLComponents(string: "https://h/p?q=a%20b&x=c%20d")!
print(c2.query ?? "nil")
print(c2.percentEncodedQuery ?? "nil")

// -- percentEncodedFragment getter --
let c3 = URLComponents(string: "https://h/p#frag%20ment")!
print(c3.fragment ?? "nil")
print(c3.percentEncodedFragment ?? "nil")

// -- percentEncodedUser / percentEncodedPassword --
let c4 = URLComponents(string: "https://user%20name:pass%20word@host.com/")!
print(c4.user ?? "nil")
print(c4.percentEncodedUser ?? "nil")
print(c4.password ?? "nil")
print(c4.percentEncodedPassword ?? "nil")

// -- percentEncodedHost / encodedHost (ASCII) --
let c5 = URLComponents(string: "https://example.com/path")!
print(c5.host ?? "nil")
print(c5.percentEncodedHost ?? "nil")
print(c5.encodedHost ?? "nil")

// -- percentEncodedQueryItems getter --
let c6 = URLComponents(string: "https://h/p?name%20here=val%20here&key=value")!
for item in c6.percentEncodedQueryItems ?? [] {
    print(item.name, item.value ?? "nil")
}
for item in c6.queryItems ?? [] {
    print(item.name, item.value ?? "nil")
}

// -- debugDescription --
let c7 = URLComponents(string: "https://host.com/path?q=1#frag")!
print(c7.debugDescription)

// -- path/query/fragment getters are decoded (regression guard) --
let c8 = URLComponents(string: "https://h/a%20b?q=hello%20world#frag%20end")!
print(c8.path)
print(c8.query ?? "nil")
print(c8.fragment ?? "nil")
print(c8.string ?? "nil")

// -- percentEncoded round-trip via URLComponents() --
var c9 = URLComponents()
c9.scheme = "https"
c9.host = "example.com"
c9.path = "/a b"
c9.query = "q=hello world"
print(c9.percentEncodedPath)
print(c9.percentEncodedQuery ?? "nil")
print(c9.string ?? "nil")
