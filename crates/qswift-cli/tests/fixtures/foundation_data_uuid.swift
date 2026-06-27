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
