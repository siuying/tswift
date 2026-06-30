import Foundation

// Data: base64, first/last, description, repeating, subdata, removeAll.
let hi = Data([72, 105])
print(hi.base64EncodedString())
print(Data(base64Encoded: "SGk=")!.count)
print(Data(base64Encoded: "%%bad%%") == nil)
print(hi.first!)
print(hi.last!)
print(hi.description)
print(Data(repeating: 7, count: 3).count)
print(hi.subdata(in: 0..<1).base64EncodedString())
var clearable = Data([1, 2, 3])
clearable.removeAll()
print(clearable.isEmpty)
print(hi == Data([72, 105]))
print(hi == Data([72, 106]))

// UUID: description + equality.
let u1 = UUID(uuidString: "E621E1F8-C36C-495A-93FC-0C247A3E6E5F")!
print(u1.description)
print(u1 == UUID(uuidString: "e621e1f8-c36c-495a-93fc-0c247a3e6e5f")!)

// URLQueryItem: name/value/description + equality.
let q = URLQueryItem(name: "key", value: "val")
print(q.name)
print(q.value!)
print(q.description)
print(q == URLQueryItem(name: "key", value: "val"))
let qNil = URLQueryItem(name: "flag", value: nil)
print(qNil.value == nil)

// URLComponents: component getters + description.
let c = URLComponents(string: "https://u:p@host.com:8080/path?a=1#frag")!
print(c.scheme!)
print(c.host!)
print(c.port!)
print(c.path)
print(c.user!)
print(c.password!)
print(c.query!)
print(c.fragment!)
print(c.description)

// URL: description, relative views, directory path, equality.
let url = URL(string: "https://example.com/a/b/")!
print(url.description)
print(url.relativePath)
print(url.relativeString)
print(url.hasDirectoryPath)
print(url.baseURL == nil)
print(url == URL(string: "https://example.com/a/b/")!)
