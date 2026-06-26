// N2 — builtin conformance accessors: description + hashValue.

// `description` matches the printed form for every builtin.
print(42.description, (1.5).description, true.description, "hi".description)
print([1, 2, 3].description)
print(Set([7]).description)
// Int-keyed dict: the runtime renders collection elements without the quoting
// Swift applies to String elements (a pre-existing print gap, tracked with the
// debugDescription follow-up), so this slice exercises the matching cases.
print([1: 2].description)

// `hashValue` is consistent within a run (the only contract; Swift seeds it
// randomly, so compare rather than print the raw integer).
print(42.hashValue == 42.hashValue, "hi".hashValue == "hi".hashValue)
print((1.5).hashValue == (1.5).hashValue, true.hashValue == true.hashValue)
print(1.hashValue == 2.hashValue)
