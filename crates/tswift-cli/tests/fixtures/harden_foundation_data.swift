import Foundation

// Harden slice 25: Data base64 and byte operations
// Ground-truth captured from Swift 6.3.2 on macOS.

// --- Standard base64 encode ---
let d1 = Data("Hello, World!".utf8)
print(d1.base64EncodedString())  // SGVsbG8sIFdvcmxkIQ==

// --- Standard base64 decode ---
let decoded = Data(base64Encoded: "SGVsbG8sIFdvcmxkIQ==")!
print(String(data: decoded, encoding: .utf8)!)  // Hello, World!

// --- invalid base64 chars (not whitespace) cause decode to return nil ---
let d3b = Data(base64Encoded: "SGVs!bG8s")
print(d3b == nil ? "nil" : "ok")  // nil

// --- Empty data ---
let empty = Data()
print(empty.base64EncodedString())   // ""
let decEmpty = Data(base64Encoded: "")!
print(decEmpty.count)  // 0

// --- Binary data round-trip ---
let bytes: [UInt8] = [0, 1, 2, 255, 254, 253]
let d6 = Data(bytes)
let enc6 = d6.base64EncodedString()
print(enc6)  // AAEC//79
let dec6 = Data(base64Encoded: enc6)!
print(Array(dec6))  // [0, 1, 2, 255, 254, 253]

// --- Missing padding makes decode fail ---
let nopad = "SGVsbG8"   // "Hello" without ==
let decNoPad = Data(base64Encoded: nopad)
print(decNoPad == nil ? "nil" : "ok")  // nil

// --- Invalid base64 chars cause nil ---
print(Data(base64Encoded: "not!valid") == nil ? "nil" : "ok")  // nil

// --- count, first, last ---
let d9 = Data([10, 20, 30])
print(d9.count)   // 3
print(d9.first!)  // 10
print(d9.last!)   // 30

// --- append byte ---
var d10 = Data([1, 2, 3])
d10.append(4)
d10.append(5)
print(Array(d10))  // [1, 2, 3, 4, 5]

// --- individual subscript access ---
let d11 = Data([10, 20, 30, 40, 50])
print(d11[0])  // 10
print(d11[4])  // 50

// --- equality ---
print(Data([1, 2, 3]) == Data([1, 2, 3]))  // true
print(Data([1, 2, 3]) == Data([1, 2, 4]))  // false

// --- hex data ---
print(Data([0xDE, 0xAD, 0xBE, 0xEF]).base64EncodedString())  // 3q2+7w==

// --- ISO8601 fractional seconds validation (iter3 follow-up fixed) ---
struct DateHolder: Codable { let date: Date }
// Use leading-dot shorthand — this is the ONLY valid Swift form.
// JSONDecoder.iso8601 does not exist in real Foundation.
var dec = JSONDecoder()
dec.dateDecodingStrategy = .iso8601

// Valid fractional seconds accepted
let jsonFrac = "{\"date\":\"2024-01-15T10:30:00.500Z\"}"
if let ev = try? dec.decode(DateHolder.self, from: Data(jsonFrac.utf8)) {
    print("fractional ok")
} else {
    print("fractional fail")
}

// Non-digit fractional suffix rejected (iter3 follow-up fixed)
let jsonAbc = "{\"date\":\"2024-01-15T10:30:00.abcZ\"}"
if let _ = try? dec.decode(DateHolder.self, from: Data(jsonAbc.utf8)) {
    print("abc accepted (BUG)")
} else {
    print("abc rejected")
}

// Empty fractional rejected
let jsonEmptyFrac = "{\"date\":\"2024-01-15T10:30:00.Z\"}"
if let _ = try? dec.decode(DateHolder.self, from: Data(jsonEmptyFrac.utf8)) {
    print("empty-frac accepted (BUG)")
} else {
    print("empty-frac rejected")
}
