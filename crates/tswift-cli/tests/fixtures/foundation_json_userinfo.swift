import Foundation

// userInfo round-trips on both coders; assumesTopLevelDictionary lets a
// brace-less top-level object decode.
struct P: Codable {
    let a: Int
    let b: Int
}

let key = CodingUserInfoKey(rawValue: "context")!
print(key.rawValue)

let enc = JSONEncoder()
enc.userInfo[key] = "hello"
print(enc.userInfo[key] as! String)

let dec = JSONDecoder()
dec.userInfo[key] = 42
print(dec.userInfo[key] as! Int)

let plistEnc = PropertyListEncoder()
plistEnc.userInfo[key] = "plist"
print(plistEnc.userInfo[key] as! String)

// assumesTopLevelDictionary: decode a brace-less top-level object.
let d2 = JSONDecoder()
d2.assumesTopLevelDictionary = true
// Note: the runtime's decode accepts a String directly (see the other
// foundation_json_* fixtures); Apple's swiftc requires Data. Semantics for
// assumesTopLevelDictionary were cross-checked against swiftc separately.
let p = try d2.decode(P.self, from: "\"a\": 1, \"b\": 2")
print(p.a, p.b)

// Still works when braces are present.
let p2 = try d2.decode(P.self, from: "{\"a\": 3, \"b\": 4}")
print(p2.a, p2.b)
