import Foundation

// harden_json.swift — JSON Codable edge cases ported from Apple's Swift test suite
// Ground-truthed against Swift 6.3.2 (2026-07-03)
// Sources: test/stdlib/CodableTests.swift, JSONEncoder/JSONDecoder Foundation tests

// --- Int.max / Int.min round-trip ---
struct IntLimits: Codable {
    var maxVal: Int
    var minVal: Int
}
var enc1 = JSONEncoder()
enc1.outputFormatting = .sortedKeys
let d1 = try! enc1.encode(IntLimits(maxVal: Int.max, minVal: Int.min))
print(String(data: d1, encoding: .utf8)!)
// {"maxVal":9223372036854775807,"minVal":-9223372036854775808}
let r1 = try! JSONDecoder().decode(IntLimits.self, from: d1)
print(r1.maxVal == Int.max)   // true
print(r1.minVal == Int.min)   // true

// --- Empty array and dict ---
struct EmptyContainers: Codable { var arr: [Int]; var dict: [String: Int] }
let d2 = try! enc1.encode(EmptyContainers(arr: [], dict: [:]))
print(String(data: d2, encoding: .utf8)!)   // {"arr":[],"dict":{}}

// --- Bool encoding ---
struct BoolStruct: Codable { var f: Bool; var t: Bool }
let d3 = try! enc1.encode(BoolStruct(f: false, t: true))
print(String(data: d3, encoding: .utf8)!)   // {"f":false,"t":true}

// --- Optional nil is omitted (not encoded as null) ---
struct OptFields: Codable { var present: Int?; var absent: Int? }
var enc2 = JSONEncoder()
let d4 = try! enc2.encode(OptFields(present: 42, absent: nil))
print(String(data: d4, encoding: .utf8)!)   // {"present":42}

// --- Deep nesting ---
struct DeepValue: Codable { var value: Int }
struct MidValue: Codable { var deep: DeepValue }
struct TopValue: Codable { var inner: MidValue }
var enc4 = JSONEncoder()
let d6 = try! enc4.encode(TopValue(inner: MidValue(deep: DeepValue(value: 42))))
print(String(data: d6, encoding: .utf8)!)   // {"inner":{"deep":{"value":42}}}

// --- Int array with extremes ---
struct IntArray: Codable { var arr: [Int] }
var enc5 = JSONEncoder()
let d7 = try! enc5.encode(IntArray(arr: [1, 2, 3, -1, Int.max, Int.min]))
print(String(data: d7, encoding: .utf8)!)
// {"arr":[1,2,3,-1,9223372036854775807,-9223372036854775808]}

// --- String escape sequences ---
struct Str: Codable { var s: String }
var enc6 = JSONEncoder()
print(String(data: try! enc6.encode(Str(s: "hello\nworld")), encoding: .utf8)!)
// {"s":"hello\nworld"}
print(String(data: try! enc6.encode(Str(s: "tab\there")), encoding: .utf8)!)
// {"s":"tab\there"}
print(String(data: try! enc6.encode(Str(s: "quote\"here")), encoding: .utf8)!)
// {"s":"quote\"here"}
print(String(data: try! enc6.encode(Str(s: "back\\here")), encoding: .utf8)!)
// {"s":"back\\here"}

// --- Double JSON encoding: scientific notation for extreme values ---
struct Dbl: Codable { var v: Double }
var enc7 = JSONEncoder()
print(String(data: try! enc7.encode(Dbl(v: 1.5e-300)), encoding: .utf8)!)  // {"v":1.5e-300}
print(String(data: try! enc7.encode(Dbl(v: 1.5e300)),  encoding: .utf8)!)  // {"v":1.5e+300}
print(String(data: try! enc7.encode(Dbl(v: -0.0)),     encoding: .utf8)!)  // {"v":-0}
print(String(data: try! enc7.encode(Dbl(v: 0.1)),      encoding: .utf8)!)  // {"v":0.1}
print(String(data: try! enc7.encode(Dbl(v: 1e-5)),     encoding: .utf8)!)  // {"v":1e-05}
print(String(data: try! enc7.encode(Dbl(v: 9.9e15)),   encoding: .utf8)!)  // {"v":9.9e+15}

// --- -0.0 encodes as -0 and decodes equal to both 0.0 and -0.0 ---
let negZeroData = Data("{\"v\":-0}".utf8)
let dnz = try! JSONDecoder().decode(Dbl.self, from: negZeroData)
print(dnz.v == -0.0)  // true
print(dnz.v == 0.0)   // true

// --- Non-finite Double throws EncodingError ---
var enc8 = JSONEncoder()
do {
    let _ = try enc8.encode(Dbl(v: Double.infinity))
    print("should have thrown")
} catch {
    print("inf throws")   // inf throws
}
do {
    let _ = try enc8.encode(Dbl(v: Double.nan))
    print("should have thrown")
} catch {
    print("nan throws")   // nan throws
}
