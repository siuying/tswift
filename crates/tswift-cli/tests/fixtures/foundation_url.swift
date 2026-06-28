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

// --- review edge cases ---
// file: scheme via URL(string:) is a file URL; empty authority preserved
let fs = URL(string: "file:///tmp/a")!
print(fs.isFileURL, fs.host ?? "nil", fs.path, fs.absoluteString)
// equality independent of how the file URL was built
print(URL(string: "file:///tmp/a")! == URL(fileURLWithPath: "/tmp/a"))
// IPv6 host keeps brackets; port parsed after ]
let v6 = URL(string: "http://[::1]:9000/x")!
print(v6.host ?? "nil", v6.port ?? -1)
// non-numeric/negative port is ignored
print(URL(string: "http://h:-1/")!.port == nil)
// percent-decoded path and query items
let pe = URL(string: "https://h/a%20b?q=hello%20world")!
print(pe.path)
let comps = URLComponents(string: "https://h/a%20b?q=hello%20world")!
print(comps.queryItems?.first?.value ?? "nil")
// dotfiles have no extension
let dot = URL(string: "file:///home/.bashrc")!
print(dot.pathExtension == "")
print(dot.deletingPathExtension().lastPathComponent)
// query is canonical: writing query updates url even without queryItems
var qc = URLComponents(string: "https://h/p")!
qc.query = "z=9"
print(qc.url?.absoluteString ?? "nil")
