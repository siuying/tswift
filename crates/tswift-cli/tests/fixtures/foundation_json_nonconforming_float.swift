import Foundation

struct M: Codable {
    let x: Double
    let y: Double
    let z: Double
}

// Encode non-finite Doubles with .convertToString.
let enc = JSONEncoder()
enc.outputFormatting = [.sortedKeys]
enc.nonConformingFloatEncodingStrategy = .convertToString(
    positiveInfinity: "+Inf", negativeInfinity: "-Inf", nan: "NaN")
let m = M(x: Double.infinity, y: -Double.infinity, z: Double.nan)
let data = try enc.encode(m)
print(String(data: data, encoding: .utf8)!)

// Decode them back with the matching .convertFromString strategy.
let dec = JSONDecoder()
dec.nonConformingFloatDecodingStrategy = .convertFromString(
    positiveInfinity: "+Inf", negativeInfinity: "-Inf", nan: "NaN")
let back = try dec.decode(M.self, from: data)
print(back.x)
print(back.y)
print(back.z.isNaN)

// Default strategy (.throw) rejects non-finite Doubles on encode.
do {
    let e2 = JSONEncoder()
    _ = try e2.encode(M(x: Double.infinity, y: 0, z: 0))
    print("no throw")
} catch {
    print("throws on infinity")
}
