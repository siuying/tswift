// Range index bounds, self-indices and hashing.
let r = 2..<7
print(r.startIndex, r.endIndex)
print(r.indices)
print(r.hashValue == (2..<7).hashValue, r.hashValue == (2..<8).hashValue)
