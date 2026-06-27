// Dictionary iteration yields a (key:, value:) element tuple; `.key` / `.value`
// member access resolves to the element's slots.
let scores = ["alice": 95, "bob": 80, "carol": 92]

let high = scores.filter { $0.value >= 90 }
print(high.count)

// Dictionary.filter returns a Dictionary, so `.keys` chains.
let topNames = scores.filter { $0.value >= 90 }.keys.sorted()
print(topNames)

for pair in scores.sorted(by: { $0.key < $1.key }) {
    print(pair.key, pair.value)
}
