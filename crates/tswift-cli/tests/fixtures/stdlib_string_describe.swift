// String description, debugDescription, hashValue, removeAll, reserveCapacity.
let s = "hi\tthere\n\"q\""
print(s.description)
print(s.debugDescription)
print("abc".hashValue == "abc".hashValue, "abc".hashValue == "abd".hashValue)

var t = "keep"
t.removeAll()
print(t.isEmpty)

var u = "grow"
u.reserveCapacity(100)
print(u)
