import Foundation

// Harden slice 25: URL edge cases
// Ground-truth captured from Swift 6.3.2 on macOS.

// --- standardized with `..` after empty segment (double slash) ---
// Previously diverged: runtime gave file:////c; now fixed.
let u1 = URL(string: "file:///a//../c")!
print(u1.standardized.absoluteString)

// `..` after double slash: pops the empty segment then `b`
let u2 = URL(string: "file:///a//b/../c")!
print(u2.standardized.absoluteString)

// Two `..` after double slash
let u3 = URL(string: "file:///a//b/../../c")!
print(u3.standardized.absoluteString)

// Double slash alone is preserved (no `..`)
let u4 = URL(string: "file:///a//b")!
print(u4.standardized.absoluteString)

// `..` clamped at root for absolute paths
let u5 = URL(string: "file:///../../a")!
print(u5.standardized.absoluteString)

// trailing `/.` preserves the trailing slash
let u6 = URL(string: "file:///a/b/./")!
print(u6.standardized.absoluteString)

// --- appendingPathExtension preserves trailing slash ---
// Previously runtime stripped the slash; now fixed.
let u7 = URL(string: "file:///a/b/")!
print(u7.appendingPathExtension("txt").absoluteString)

// No trailing slash: no slash added
let u8 = URL(string: "file:///a/foo")!
print(u8.appendingPathExtension("txt").absoluteString)

// Multiple extensions: removes only the last one
let u9 = URL(string: "file:///a/file.tar.gz")!
print(u9.deletingPathExtension().absoluteString)

// --- appendingPathComponent ---
// Empty component appends trailing slash
let u10 = URL(string: "file:///a")!
print(u10.appendingPathComponent("").absoluteString)

// Component with space is percent-encoded
let u11 = URL(string: "file:///a")!
print(u11.appendingPathComponent("hello world").absoluteString)

// --- pathComponents ---
let u12 = URL(string: "file:///a/b/c")!
let comps = u12.pathComponents
print(comps.count)
print(comps[0])
print(comps[1])
print(comps[2])
print(comps[3])

// --- hasDirectoryPath ---
print(URL(string: "file:///a/b/")!.hasDirectoryPath)
print(URL(string: "file:///a/b")!.hasDirectoryPath)

// --- dotfile has no extension ---
let u13 = URL(string: "file:///a/.hidden")!
print(u13.pathExtension == "" ? "empty" : u13.pathExtension)

// --- HTTP URL standardized ---
let u14 = URL(string: "https://example.com/a/b/../c")!
print(u14.standardized.absoluteString)

// --- fileURLWithPath ---
let u15 = URL(fileURLWithPath: "/tmp/test.txt")
print(u15.absoluteString)

// --- isFileURL ---
print(URL(string: "file:///a")!.isFileURL)
print(URL(string: "https://example.com")!.isFileURL)
