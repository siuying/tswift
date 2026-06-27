// `#available` / `#unavailable` conditions. The runtime targets one current
// platform, so availability is always satisfied.
if #available(macOS 10.15, iOS 13, *) {
    print("available")
} else {
    print("unavailable")
}

if #unavailable(iOS 13) {
    print("legacy")
} else {
    print("modern")
}

func guarded() {
    guard #available(iOS 13, *) else {
        print("skip")
        return
    }
    print("run")
}
guarded()

let combined = #available(iOS 13, *) && true
print(combined)
