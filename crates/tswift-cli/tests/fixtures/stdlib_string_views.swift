// String UTF-8 / UTF-16 / Unicode scalar views.
print("AB".utf8.count)
print(Array("AB".utf8))
print("é".utf8.count)
print("AB".utf16.count)
print("AB".unicodeScalars.count)
print(Array("AB".unicodeScalars))

var total = 0
for b in "AB".utf8 { total += Int(b) }
print(total)
