import Foundation

// Codable structs exercising URL, UUID, and Data JSON coding.

struct WithURL: Codable {
    let endpoint: URL
}

struct WithUUID: Codable {
    let id: UUID
}

struct WithData: Codable {
    let payload: Data
}

struct URLList: Codable {
    let urls: [URL]
}

struct WithOptUUID: Codable {
    let id: UUID?
}

struct DataList: Codable {
    let chunks: [Data]
}

// 1. URL round-trip: absoluteString as JSON string
let url = URL(string: "https://example.com/api")!
let w1 = WithURL(endpoint: url)
var enc1 = JSONEncoder()
let d1 = try enc1.encode(w1)
print(String(data: d1, encoding: .utf8)!)
let dec1 = JSONDecoder()
let w2 = try dec1.decode(WithURL.self, from: d1)
print(w2.endpoint.absoluteString)

// 2. UUID round-trip: uuidString as JSON string
let fixedUUID = UUID(uuidString: "550E8400-E29B-41D4-A716-446655440000")!
let w3 = WithUUID(id: fixedUUID)
var enc2 = JSONEncoder()
let d3 = try enc2.encode(w3)
print(String(data: d3, encoding: .utf8)!)
let dec2 = JSONDecoder()
let w4 = try dec2.decode(WithUUID.self, from: d3)
print(w4.id.uuidString)

// 3. Malformed UUID string throws dataCorrupted
do {
    _ = try dec2.decode(WithUUID.self, from: "{\"id\":\"not-a-uuid\"}")
} catch {
    print("invalid UUID throws")
}

// 4. Data base64 round-trip (default strategy = .base64)
let bytes = Data([72, 101, 108, 108, 111])
let w5 = WithData(payload: bytes)
var enc3 = JSONEncoder()
let d5 = try enc3.encode(w5)
print(String(data: d5, encoding: .utf8)!)
let dec3 = JSONDecoder()
let w6 = try dec3.decode(WithData.self, from: d5)
print(w6.payload.count)

// 5. Invalid base64 string throws dataCorrupted
do {
    _ = try dec3.decode(WithData.self, from: "{\"payload\":\"!!!invalid\"}")
} catch {
    print("invalid base64 throws")
}

// 6. dataEncodingStrategy = .deferredToData → array of byte integers
var enc4 = JSONEncoder()
enc4.dataEncodingStrategy = .deferredToData
let d6 = try enc4.encode(w5)
print(String(data: d6, encoding: .utf8)!)

// 7a. Invalid URL string in JSON throws dataCorrupted
do {
    _ = try dec1.decode(WithURL.self, from: "{\"endpoint\":\"http://exa mple.com\"}")
} catch {
    print("invalid URL throws")
}

// 7b. [URL] array: each URL encodes as its absoluteString
let w7 = URLList(urls: [URL(string: "https://a.com")!, URL(string: "https://b.com")!])
var enc5 = JSONEncoder()
let d7 = try enc5.encode(w7)
print(String(data: d7, encoding: .utf8)!)
let dec5 = JSONDecoder()
let w8 = try dec5.decode(URLList.self, from: d7)
print(w8.urls.count)
print(w8.urls[0].absoluteString)

// 8. UUID? — encode nil and present; decode null and valid string
var enc6 = JSONEncoder()
let w9 = WithOptUUID(id: fixedUUID)
let d9 = try enc6.encode(w9)
print(String(data: d9, encoding: .utf8)!)
let w10 = WithOptUUID(id: nil)
let d10 = try enc6.encode(w10)
print(String(data: d10, encoding: .utf8)!)
let dec6 = JSONDecoder()
let w11 = try dec6.decode(WithOptUUID.self, from: "{\"id\":null}")
print(w11.id == nil)
let w12 = try dec6.decode(WithOptUUID.self, from: "{\"id\":\"550E8400-E29B-41D4-A716-446655440000\"}")
print(w12.id?.uuidString ?? "nil")

// 9. deferredToData decode: int array → Data
var dec7 = JSONDecoder()
dec7.dataDecodingStrategy = .deferredToData
let w13 = try dec7.decode(WithData.self, from: "{\"payload\":[72,101,108,108,111]}")
print(w13.payload.count)

// 10. [Data] round-trip: array of base64 strings
let dc8 = DataList(chunks: [Data([72, 101, 108]), Data([108, 111])])
var enc7 = JSONEncoder()
let d8 = try enc7.encode(dc8)
print(String(data: d8, encoding: .utf8)!)
let dec8 = JSONDecoder()
let w14 = try dec8.decode(DataList.self, from: d8)
print(w14.chunks.count)
print(w14.chunks[0].count)
