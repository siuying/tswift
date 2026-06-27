// MemoryLayout<T>.size / .stride / .alignment must parse and type-check. The
// `<T>` argument is a written type recorded for the runtime.
// expected-no-diagnostics

struct Point {
    var x: Int
    var y: Int
}

let s = MemoryLayout<Int>.size
let t = MemoryLayout<Point>.stride
let a = MemoryLayout<Double>.alignment
let _ = (s, t, a)
