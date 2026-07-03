import Foundation

// UUID Comparable (since macOS 13)
let u1 = UUID(uuidString: "10000000-0000-0000-0000-000000000000")!
let u2 = UUID(uuidString: "20000000-0000-0000-0000-000000000000")!
let u3 = UUID(uuidString: "10000000-0000-0000-0000-000000000000")!
print(u1 < u2)
print(u2 < u1)
print(u1 < u3)
print(u1 <= u3)
print(u2 > u1)
print(u1 >= u3)

// UUID encode (Codable — encodes as uuidString)
let u4 = UUID(uuidString: "E2B8BE3F-4C7D-41F3-8D5F-B8D43C343111")!
let enc = try! JSONEncoder().encode(u4)
print(String(data: enc, encoding: .utf8)!)
