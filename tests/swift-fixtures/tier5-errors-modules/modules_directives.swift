// rust-gap: advanced Tier 0-10 spec syntax not yet modelled by the pure-Rust frontend (tracked in #37)
// expected-no-diagnostics
// Tier 5 / 9d — import, #if conditional compilation, @main entry point.

import Foundation

#if DEBUG
let mode = "debug"
#else
let mode = "release"
#endif

#if os(macOS) || os(Linux)
let platform = "desktop"
#else
let platform = "other"
#endif

@main
struct App {
    static func main() {
        print("running in \(mode) on \(platform)")
    }
}