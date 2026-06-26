// Bool — toggle, description, hashValue.
var flag = true
flag.toggle()
print(flag)
flag.toggle()
print(flag)

print(true.description, false.description)

var count = 0
for value in [true, false, true] where value {
    count += 1
}
print(count)

print(true.hashValue == true.hashValue, true.hashValue == false.hashValue)
