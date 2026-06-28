// Int double-width multiply/divide and the words view.
let m = (1 << 40).multipliedFullWidth(by: 1 << 40)
print(m.high, m.low)

let m2 = (1000).multipliedFullWidth(by: 1000)
print(m2.high, m2.low)

let d = (10).dividingFullWidth((high: 0, low: 100))
print(d.quotient, d.remainder)

print((255).words)
print(Int8(-1).words, UInt8(255).words)
