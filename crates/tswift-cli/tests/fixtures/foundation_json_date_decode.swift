import Foundation

struct Event: Codable {
    let name: String
    let at: Date
}

// 1. deferredToDate: decode from timeIntervalSinceReferenceDate double
// ref=86400 → timeIntervalSinceReferenceDate should be 86400
let dec1 = JSONDecoder()
let ev1 = try dec1.decode(Event.self, from: "{\"name\":\"ref\",\"at\":86400.0}")
print(ev1.name)
print(ev1.at.timeIntervalSinceReferenceDate)

// 2. secondsSince1970: decode from Unix seconds (978393600 = ref+86400)
var dec2 = JSONDecoder()
dec2.dateDecodingStrategy = .secondsSince1970
let ev2 = try dec2.decode(Event.self, from: "{\"name\":\"s70\",\"at\":978393600.0}")
print(ev2.name)
print(ev2.at.timeIntervalSinceReferenceDate)

// 3. millisecondsSince1970: decode from Unix milliseconds (978393600000)
var dec3 = JSONDecoder()
dec3.dateDecodingStrategy = .millisecondsSince1970
let ev3 = try dec3.decode(Event.self, from: "{\"name\":\"ms70\",\"at\":978393600000.0}")
print(ev3.name)
print(ev3.at.timeIntervalSinceReferenceDate)

// 4. iso8601: decode from ISO 8601 string
var dec4 = JSONDecoder()
dec4.dateDecodingStrategy = .iso8601
let ev4 = try dec4.decode(Event.self, from: "{\"name\":\"iso\",\"at\":\"2001-01-02T00:00:00Z\"}")
print(ev4.name)
print(ev4.at.timeIntervalSinceReferenceDate)

// 5. iso8601 with malformed string throws dataCorrupted
do {
    var dec5 = JSONDecoder()
    dec5.dateDecodingStrategy = .iso8601
    _ = try dec5.decode(Event.self, from: "{\"name\":\"bad\",\"at\":\"not-a-date\"}")
} catch {
    print("iso8601 malformed throws")
}

// 6. iso8601 with fractional seconds (should parse, truncating fraction)
var dec6 = JSONDecoder()
dec6.dateDecodingStrategy = .iso8601
let ev6 = try dec6.decode(Event.self, from: "{\"name\":\"frac\",\"at\":\"2001-01-02T00:00:00.500Z\"}")
print(ev6.name)
print(ev6.at.timeIntervalSinceReferenceDate)

// 7. iso8601 without trailing Z throws (Z is required for UTC)
do {
    var dec7 = JSONDecoder()
    dec7.dateDecodingStrategy = .iso8601
    _ = try dec7.decode(Event.self, from: "{\"name\":\"noz\",\"at\":\"2001-01-02T00:00:00\"}")
} catch {
    print("iso8601 missing Z throws")
}

// 8. iso8601 with February 31 throws (invalid calendar date)
do {
    var dec8 = JSONDecoder()
    dec8.dateDecodingStrategy = .iso8601
    _ = try dec8.decode(Event.self, from: "{\"name\":\"feb31\",\"at\":\"2024-02-31T00:00:00Z\"}")
} catch {
    print("iso8601 Feb 31 throws")
}

// 9. iso8601 with Feb 29 in non-leap year throws
do {
    var dec9 = JSONDecoder()
    dec9.dateDecodingStrategy = .iso8601
    _ = try dec9.decode(Event.self, from: "{\"name\":\"leap\",\"at\":\"2023-02-29T00:00:00Z\"}")
} catch {
    print("iso8601 non-leap Feb 29 throws")
}
