import Foundation

// Harden slice 25: URLComponents edge cases
// Ground-truth captured from Swift 6.3.2 on macOS.

// --- Basic round-trip through URLComponents(string:) ---
var c1 = URLComponents(string: "https://user:pass@example.com:8080/path?q=1&r=2#frag")!
print(c1.scheme!)
print(c1.user!)
print(c1.host!)
print(c1.port!)
print(c1.path)
print(c1.query!)
print(c1.fragment!)

// --- Set decoded path, read url and percentEncodedPath ---
var c2 = URLComponents()
c2.scheme = "https"
c2.host = "example.com"
c2.path = "/hello world"
print(c2.url!.absoluteString)
print(c2.percentEncodedPath)
print(c2.path)

// --- queryItems getter decodes percent-escapes ---
var c3 = URLComponents(string: "https://example.com?a=1&b=hello%20world")!
print(c3.query!)
let items = c3.queryItems!
print(items[0].name)
print(items[0].value!)
print(items[1].name)
print(items[1].value!)

// --- queryItems setter: builds percent-encoded query in URL ---
var c4 = URLComponents()
c4.scheme = "https"
c4.host = "example.com"
c4.queryItems = [URLQueryItem(name: "q", value: "hello world"), URLQueryItem(name: "n", value: "1")]
print(c4.url!.absoluteString)

// --- percentEncodedQuery getter / query getter ---
var c5 = URLComponents(string: "https://example.com?q=hello%20world")!
print(c5.percentEncodedQuery!)
print(c5.query!)

// --- fragment with special chars ---
var c6 = URLComponents()
c6.scheme = "https"
c6.host = "example.com"
c6.fragment = "section 1"
print(c6.url!.absoluteString)

// --- query item with nil value ---
var c7 = URLComponents()
c7.scheme = "https"
c7.host = "example.com"
c7.queryItems = [URLQueryItem(name: "flag", value: nil)]
print(c7.url!.absoluteString)

// --- percentEncodedPath round-trip via setter ---
var c8 = URLComponents()
c8.scheme = "file"
c8.host = ""
c8.percentEncodedPath = "/hello%20world"
print(c8.path)
print(c8.url!.absoluteString)

// --- port nil when absent ---
var c9 = URLComponents(string: "https://example.com/path")!
print(c9.port == nil ? "nil" : "\(c9.port!)")
