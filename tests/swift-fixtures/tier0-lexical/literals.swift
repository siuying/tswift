// expected-no-diagnostics
// Tier 0 — integer / float / bool / nil / string literals and their forms.

let decimal = 1_000_000
let hex = 0xFF
let octal = 0o755
let binary = 0b1010_1010

let pi = 3.14159
let scaled = 1.5e3
let hexFloat = 0x1.8p1

let truthy = true
let falsy = false

let absent: Int? = nil

let escapes = "tab\tnl\nquote\"backslash\\unicode\u{1F600}"
let multiline = """
    first line
    second line
    """
let rawString = #"a path C:\temp has no \n escape"#
let interpolated = "decimal + hex = \(decimal + hex)"
