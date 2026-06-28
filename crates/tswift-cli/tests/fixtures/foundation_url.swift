import Foundation

// URL parsing accessors
let u = URL(string: "https://user:pass@example.com:8080/path/to/file.txt?q=1&x=2#frag")!
print(u.scheme ?? "nil")
print(u.host ?? "nil")
print(u.port ?? -1)
print(u.user ?? "nil")
print(u.password ?? "nil")
print(u.path)
print(u.query ?? "nil")
print(u.fragment ?? "nil")
print(u.lastPathComponent)
print(u.pathExtension)
print(u.pathComponents)
print(u.absoluteString)
print(u.isFileURL)

// Failable init
print(URL(string: "") == nil)

// file URLs
let f = URL(fileURLWithPath: "/tmp/data.json")
print(f.isFileURL, f.scheme ?? "nil", f.path, f.pathExtension)

// Path manipulation
let base = URL(string: "https://a.com/dir")!
print(base.appendingPathComponent("sub").absoluteString)
print(base.appendingPathExtension("zip").absoluteString)
print(base.deletingLastPathComponent().absoluteString)
print(URL(string: "https://a.com/a/b.txt")!.deletingPathExtension().absoluteString)

// Mutating path manipulation
var m = URL(string: "https://a.com/x")!
m.appendPathComponent("y")
print(m.absoluteString)
m.deleteLastPathComponent()
print(m.absoluteString)

// Equality
print(URL(string: "https://a.com")! == URL(string: "https://a.com")!)

// URLQueryItem
let q = URLQueryItem(name: "key", value: "val")
print(q.name, q.value ?? "nil")
print(URLQueryItem(name: "k", value: nil).value == nil)
print(q == URLQueryItem(name: "key", value: "val"))

// URLComponents read
var c = URLComponents(string: "https://host.com/p?a=1&b=2")!
print(c.scheme ?? "nil", c.host ?? "nil", c.path)
print(c.queryItems?.count ?? -1)

// URLComponents mutate and rebuild
c.scheme = "http"
print(c.url?.absoluteString ?? "nil")

// URLComponents from scratch
var empty = URLComponents()
empty.scheme = "https"
empty.host = "x.com"
empty.path = "/y"
print(empty.string ?? "nil")
