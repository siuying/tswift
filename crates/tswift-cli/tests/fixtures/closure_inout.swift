// Closures with `inout` parameters: the closure mutates the caller's variable
// through `&`, and the change is written back.
func applyTwice(_ start: Int, _ f: (inout Int) -> Void) -> Int {
    var x = start
    f(&x)
    f(&x)
    return x
}
var step = 5
print(applyTwice(10) { (v: inout Int) in v += step })

// A closure value with multiple parameters, one of them `inout`.
let combine = { (acc: inout Int, x: Int) in acc += x }
var total = 0
combine(&total, 5)
combine(&total, 7)
print(total)

// An `inout` closure stored in a variable, then applied.
func transform(_ value: Int, using f: (inout Int) -> Void) -> Int {
    var v = value
    f(&v)
    return v
}
let doubler: (inout Int) -> Void = { (n: inout Int) in n *= 2 }
print(transform(21, using: doubler))

// `inout` writeback happens even when the closure throws.
struct Boom: Error {}
func bumpThenThrow(_ value: Int, _ f: (inout Int) throws -> Void) -> Int {
    var v = value
    do {
        try f(&v)
    } catch {
        // swallow; v should still reflect the mutation before the throw
    }
    return v
}
print(bumpThenThrow(100) { (n: inout Int) in
    n += 1
    throw Boom()
})
