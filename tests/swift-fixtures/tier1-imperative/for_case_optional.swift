// oracle-gap: the C msf rejects the optional pattern `?` inside `for case`.
// Tier 1c — iterate the non-nil elements of an array of optionals.

let values: [Int?] = [1, nil, 3, nil, 5]
var present: [Int] = []
for case let value? in values {
    present.append(value)
}
let _ = present
