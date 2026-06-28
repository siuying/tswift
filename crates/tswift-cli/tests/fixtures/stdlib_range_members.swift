// Range description, overlaps and distance.
print((1..<5).description)
print((1...5).description)
print((1..<5).overlaps(3..<8))
print((1..<5).overlaps(6..<8))
print((1..<5).overlaps(5..<8))
print((1...5).overlaps(5...8))
print((0..<10).distance(from: 2, to: 7))
