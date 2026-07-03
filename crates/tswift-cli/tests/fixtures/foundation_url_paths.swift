import Foundation

// ── appending(path:) ───────────────────────────────────────────────────────
let base = URL(string: "https://a.com/dir")!
// basic append
print(base.appending(path: "sub").absoluteString)
// multi-segment
print(base.appending(path: "sub/file.txt").absoluteString)
// leading slash is stripped, remainder is appended (does NOT replace path)
print(base.appending(path: "/other").absoluteString)
// only the FIRST leading slash is stripped; second slash is preserved
print(base.appending(path: "//other").absoluteString)
// empty path appends a trailing slash (Foundation semantics)
print(base.appending(path: "").absoluteString)

// ── appending(component:) ──────────────────────────────────────────────────
print(base.appending(component: "sub").absoluteString)
// spaces are percent-encoded
print(base.appending(component: "hello world").absoluteString)
// slash inside component is percent-encoded as %2F (opaque single component)
print(base.appending(component: "a/b").absoluteString)

// ── append(path:) – mutating ───────────────────────────────────────────────
var m1 = URL(string: "https://a.com/dir")!
m1.append(path: "sub")
print(m1.absoluteString)

// ── append(component:) – mutating ─────────────────────────────────────────
var m2 = URL(string: "https://a.com/dir")!
m2.append(component: "sub")
print(m2.absoluteString)

// ── appendPathExtension(_:) – mutating ────────────────────────────────────
var m3 = URL(string: "file:///a/b")!
m3.appendPathExtension("txt")
print(m3.absoluteString)

// empty extension is a no-op
var m4 = URL(string: "file:///a/b")!
m4.appendPathExtension("")
print(m4.absoluteString)

// ── deletePathExtension() – mutating ──────────────────────────────────────
var m5 = URL(string: "file:///a/b.txt")!
m5.deletePathExtension()
print(m5.absoluteString)

// dotfile has no extension – no-op
var m6 = URL(string: "file:///home/.bashrc")!
m6.deletePathExtension()
print(m6.absoluteString)

// ── standardized (property) ────────────────────────────────────────────────
// resolve ".."
print(URL(string: "file:///a/b/../c")!.standardized.absoluteString)
// resolve "."
print(URL(string: "file:///a/./b")!.standardized.absoluteString)
// double slash is PRESERVED (standardized only resolves . and ..)
print(URL(string: "file:///a//b")!.standardized.absoluteString)
// ".." at root is clamped — path never collapses below "/"
print(URL(string: "file:///../../a")!.standardized.absoluteString)
// single ".." at root → path must be "/" not ""
print(URL(string: "file:///.." )!.standardized.absoluteString)
// multiple ".." at root → also "/"
print(URL(string: "file:///../..")!.standardized.absoluteString)
// non-file URL also standardized
print(URL(string: "https://a.com/x/y/../z")!.standardized.absoluteString)

// ── standardize() – mutating ──────────────────────────────────────────────
var m7 = URL(string: "file:///a/b/../c")!
m7.standardize()
print(m7.absoluteString)

// ── absoluteURL (property) ─────────────────────────────────────────────────
let abs1 = URL(string: "https://a.com/x")!
print(abs1.absoluteURL.absoluteString)

let abs2 = URL(fileURLWithPath: "/tmp/foo")
print(abs2.absoluteURL.absoluteString)

// ── resolvingSymlinksInPath() ──────────────────────────────────────────────
// for file URLs: lexical standardization (no real filesystem calls)
print(URL(string: "file:///a/b/../c")!.resolvingSymlinksInPath().absoluteString)
// lexical only — /private prefix is NOT stripped (no real filesystem call)
print(URL(string: "file:///private/tmp/a")!.resolvingSymlinksInPath().absoluteString)
// non-file URL: lexical standardization only
print(URL(string: "https://a.com/x/../y")!.resolvingSymlinksInPath().absoluteString)

// ── resolveSymlinksInPath() – mutating ─────────────────────────────────────
var m8 = URL(string: "file:///a/b/../c")!
m8.resolveSymlinksInPath()
print(m8.absoluteString)
