import Foundation

var bytes = Data([65, 66, 67])
print(bytes.count)
print(bytes.isEmpty)
bytes.append(68)
print(bytes.count)

let id = UUID(uuidString: "e2b8be3f-4c7d-41f3-8d5f-b8d43c343111")
print(id.uuidString)
let invalid = UUID(uuidString: "not-a-uuid")
print(invalid == nil)

// Hashable / debug rendering.
print(bytes.debugDescription)
let d1 = Data([1, 2, 3])
let d2 = Data([1, 2, 3])
print(d1.hashValue == d2.hashValue)
let u1 = UUID(uuidString: "e2b8be3f-4c7d-41f3-8d5f-b8d43c343111")!
let u2 = UUID(uuidString: "E2B8BE3F-4C7D-41F3-8D5F-B8D43C343111")!
print(u1.hashValue == u2.hashValue)
print(u1.debugDescription)
