// Optional equality against wrapped values, nil, and other optionals.
let a: Int? = 5
let b: Int? = nil
print(a == 5, a != nil, b == nil, b != 5)
print(a == b, a != b)

let s: String? = "hi"
print(s == "hi", s != nil)
