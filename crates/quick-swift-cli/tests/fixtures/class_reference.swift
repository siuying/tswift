class Counter { var count = 0 }
let a = Counter()
let b = a
b.count = 5
print(a.count, b.count)
print(a === b)
let c = Counter()
print(a === c)
