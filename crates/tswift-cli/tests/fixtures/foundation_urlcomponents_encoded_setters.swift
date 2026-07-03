import Foundation

// -- percentEncodedPath setter --
var c1 = URLComponents()
c1.scheme = "https"
c1.host = "h"
c1.percentEncodedPath = "/a%20b"
print(c1.path)
print(c1.percentEncodedPath)
print(c1.string ?? "nil")

// -- percentEncodedQuery setter --
var c2 = URLComponents()
c2.scheme = "https"
c2.host = "h"
c2.percentEncodedQuery = "q=hello%20world"
print(c2.query ?? "nil")
print(c2.percentEncodedQuery ?? "nil")
print(c2.string ?? "nil")

// -- percentEncodedFragment setter --
var c3 = URLComponents()
c3.scheme = "https"
c3.host = "h"
c3.path = "/"
c3.percentEncodedFragment = "frag%20ment"
print(c3.fragment ?? "nil")
print(c3.percentEncodedFragment ?? "nil")
print(c3.string ?? "nil")

// -- percentEncodedUser + percentEncodedPassword setter --
var c4 = URLComponents()
c4.percentEncodedUser = "user%20name"
c4.percentEncodedPassword = "pass%20word"
c4.host = "host.com"
c4.path = "/"
print(c4.user ?? "nil")
print(c4.percentEncodedUser ?? "nil")
print(c4.password ?? "nil")
print(c4.percentEncodedPassword ?? "nil")
print(c4.string ?? "nil")

// -- percentEncodedQueryItems setter --
var c5 = URLComponents()
c5.scheme = "https"
c5.host = "h"
c5.percentEncodedQueryItems = [URLQueryItem(name: "hello%20world", value: "foo%20bar")]
print(c5.queryItems?.first?.name ?? "nil")
print(c5.queryItems?.first?.value ?? "nil")
print(c5.percentEncodedQueryItems?.first?.name ?? "nil")
print(c5.percentEncodedQueryItems?.first?.value ?? "nil")
print(c5.query ?? "nil")
print(c5.percentEncodedQuery ?? "nil")
print(c5.string ?? "nil")

// -- percentEncodedHost setter: decodes and canonicalises (%61 -> 'a') --
var c6 = URLComponents()
c6.percentEncodedHost = "ex%61mple.com"
print(c6.host ?? "nil")
print(c6.percentEncodedHost ?? "nil")
print(c6.encodedHost ?? "nil")

// -- encodedHost setter: preserves verbatim encoded form --
var c7 = URLComponents()
c7.encodedHost = "ex%61mple.com"
print(c7.host ?? "nil")
print(c7.percentEncodedHost ?? "nil")
print(c7.encodedHost ?? "nil")

// -- encodedHost in URL string --
var c8 = URLComponents()
c8.scheme = "https"
c8.encodedHost = "ex%61mple.com"
c8.path = "/"
print(c8.string ?? "nil")

// -- percentEncodedHost in URL string (canonical) --
var c9 = URLComponents()
c9.scheme = "https"
c9.percentEncodedHost = "ex%61mple.com"
c9.path = "/"
print(c9.string ?? "nil")

// -- plain host setter clears encodedHost override --
var c10 = URLComponents()
c10.encodedHost = "ex%61mple.com"
c10.host = "other.com"
print(c10.host ?? "nil")
print(c10.encodedHost ?? "nil")

// -- encodedHost nil clears host --
var c11 = URLComponents()
c11.encodedHost = "example.com"
c11.encodedHost = nil
print(c11.host ?? "nil")
print(c11.encodedHost ?? "nil")
