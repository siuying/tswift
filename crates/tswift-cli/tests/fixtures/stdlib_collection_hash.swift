// Order-independent hashing for Set and Dictionary.
let s1: Set = [1, 2, 3]
let s2: Set = [3, 2, 1]
print(s1.hashValue == s2.hashValue, s1.hashValue == Set([1, 2]).hashValue)

let d1 = ["a": 1, "b": 2]
let d2 = ["b": 2, "a": 1]
print(d1.hashValue == d2.hashValue, d1.hashValue == ["a": 1].hashValue)
