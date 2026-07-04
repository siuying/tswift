import Foundation

// Phase 4: JSONEncoder/JSONDecoder are class-backed Objects — `let`-bound
// encoder/decoder can have properties set without `var`.

struct Point: Codable {
    let x: Int
    let y: Int
}

// 1. let-bound encoder: outputFormatting = .prettyPrinted
let encoder = JSONEncoder()
encoder.outputFormatting = .prettyPrinted
let data = try encoder.encode(Point(x: 3, y: 7))
print(String(data: data, encoding: .utf8)!)

// 2. Overwrite outputFormatting on same let-bound encoder — compact output.
encoder.outputFormatting = .sortedKeys
let data2 = try encoder.encode(Point(x: 1, y: 2))
print(String(data: data2, encoding: .utf8)!)

// 3. Alias shares the same Object — mutation through alias is visible via encoder.
let enc2 = encoder
enc2.outputFormatting = .prettyPrinted
let data3 = try encoder.encode(Point(x: 5, y: 9))
print(String(data: data3, encoding: .utf8)!)

// 4. let-bound decoder: keyDecodingStrategy = .convertFromSnakeCase
let decoder = JSONDecoder()
decoder.keyDecodingStrategy = .convertFromSnakeCase
