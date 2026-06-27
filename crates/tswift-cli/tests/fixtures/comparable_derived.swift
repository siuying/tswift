// A Comparable type defines only `static func <`; the runtime derives
// >, <=, >= and powers min/max/sorted from it.

struct Version: Comparable {
    let major: Int
    let minor: Int
    static func < (a: Version, b: Version) -> Bool {
        if a.major != b.major { return a.major < b.major }
        return a.minor < b.minor
    }
    static func == (a: Version, b: Version) -> Bool {
        a.major == b.major && a.minor == b.minor
    }
}

let a = Version(major: 1, minor: 2)
let b = Version(major: 1, minor: 5)
print(a < b)
print(a > b)
print(a <= b)
print(a >= b)
print(b >= a)
print(min(a, b) == a)
print(max(a, b) == b)
print(max(a, b, Version(major: 2, minor: 0)) == Version(major: 2, minor: 0))
print([b, a].sorted().first! == a)
