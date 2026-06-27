// expected-no-diagnostics
import Foundation

var bytes = Data([65, 66, 67])
bytes.append(68)
let byteCount = bytes.count
let identifier = UUID(uuidString: "e2b8be3f-4c7d-41f3-8d5f-b8d43c343111")
let text = identifier.uuidString
