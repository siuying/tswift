import Foundation

struct Point: Codable {
    let x: Int
    let y: Int
}

struct Config: Codable {
    let name: String
    let value: Int
    let active: Bool
}

// 1. prettyPrinted alone — default key order, 2-space indent, " : " separator
var enc1 = JSONEncoder()
enc1.outputFormatting = .prettyPrinted
let d1 = try enc1.encode(Point(x: 1, y: 2))
print(String(data: d1, encoding: .utf8)!)

// 2. sortedKeys alone — compact, keys in lexicographic order
var enc2 = JSONEncoder()
enc2.outputFormatting = .sortedKeys
let d2 = try enc2.encode(Config(name: "test", value: 42, active: true))
print(String(data: d2, encoding: .utf8)!)

// 3. prettyPrinted + sortedKeys — pretty with sorted keys
var enc3 = JSONEncoder()
enc3.outputFormatting = [.prettyPrinted, .sortedKeys]
let d3 = try enc3.encode(Config(name: "alpha", value: 7, active: false))
print(String(data: d3, encoding: .utf8)!)
