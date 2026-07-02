class A {}
class B: A {}
let b = B()
let opt = b as A?
print(opt != nil)
let n = 5 as Int?
print(n ?? -1)
let val: Any = "s"
print((val as? String) ?? "no")
