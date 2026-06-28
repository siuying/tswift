// Optional unsafelyUnwrapped and hashValue on present optionals.
let a: Int? = 5
print(a.unsafelyUnwrapped)
print(a.hashValue == (5).hashValue)

let s: String? = "hi"
print(s.unsafelyUnwrapped)
print(s.hashValue == "hi".hashValue)
