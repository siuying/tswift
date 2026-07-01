// Optional-chained calls `f?()` and subscripts `a?[i]`: nil short-circuits
// without evaluating arguments/indices; ternaries with parenthesized branches
// are unaffected by the adjacency-based disambiguation.

func effect(_ tag: String) -> Int {
  print("effect \(tag)")
  return 1
}

// nil callee: the call and its argument side effects are skipped.
var handler: ((Int) -> Void)? = nil
handler?(effect("skipped"))
print("no effect yet")

handler = { n in print("handled \(n)") }
handler?(effect("ran"))

// nil base subscript: index side effects are skipped.
let missing: [Int]? = nil
_ = missing?[effect("index skipped")]
print("still no index effect")

let present: [Int]? = [10, 20]
print(present?[effect("index ran")] ?? -1)

// Ternary with parenthesized branches is not an optional call.
let flag = true
let t1 = flag ? (1 + 1) : (2 + 2)
print(t1)

// Optional-chained method call on an optional value.
let word: String? = "swift"
print(word?.hasPrefix("sw") ?? false)
let none: String? = nil
print(none?.hasPrefix("sw") ?? false)
