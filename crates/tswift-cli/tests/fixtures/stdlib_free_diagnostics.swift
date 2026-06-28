// Free functions — readLine, assertionFailure, preconditionFailure.
// The harness provides no stdin, so readLine() yields nil.
let line = readLine()
print(line ?? "no-input")

// Diagnostics guarded behind branches that never execute, so the program
// still terminates normally while exercising the registered intrinsics.
let value = 21 * 2
if value != 42 {
    assertionFailure("arithmetic is broken")
}
if value < 0 {
    preconditionFailure("value must be non-negative")
}
print(value)
