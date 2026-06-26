// Regex literals — /.../ and extended #/.../#  (Swift 5.7+)
let log = "2024-01-15 ERROR src/disk.swift / 2024-01-16 WARN src/mem.swift"

// Match every date with a reusable literal.
let date = /\d{4}-\d{2}-\d{2}/
let dates = log.matches(of: date).map { $0.0 }
print("dates: \(dates.joined(separator: ", "))")

// Capture groups: whole match at .0, groups after it.
if let entry = log.firstMatch(of: /(\d{4})-(\d{2})-(\d{2}) (\w+)/) {
    print("first entry → year \(entry.1), level \(entry.4)")
}

// Validate a whole string with anchors + alternation.
func isLevel(_ s: String) -> Bool {
    return s.wholeMatch(of: /ERROR|WARN|INFO/) != nil
}
print("ERROR is a level: \(isLevel("ERROR"))")
print("OOPS  is a level: \(isLevel("OOPS"))")

// Replace all matches.
let redacted = log.replacing(/\d{4}-\d{2}-\d{2}/, with: "<date>")
print("redacted: \(redacted)")

// Extended #/.../# keeps "/" literal — handy for file paths.
let paths = log.matches(of: #/\w+\/\w+\.\w+/#).map { $0.0 }
print("paths: \(paths.joined(separator: ", "))")
