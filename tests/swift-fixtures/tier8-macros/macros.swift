// oracle-gap: macros and result builders (Swift 5.9+) are an F8+ frontend gap;
// the C msf does not parse macro declarations or result-builder transforms.
// Tier 8 — macro declarations, @resultBuilder, and built-in directives.

@freestanding(expression)
macro stringify<T>(_ value: T) -> (T, String) =
    #externalMacro(module: "MacrosPlugin", type: "StringifyMacro")

@attached(member)
macro AddInit() = #externalMacro(module: "MacrosPlugin", type: "AddInitMacro")

@resultBuilder
struct StringBuilder {
    static func buildBlock(_ parts: String...) -> String {
        parts.joined(separator: " ")
    }
}

@StringBuilder
func greeting() -> String {
    "Hello"
    "World"
}

let currentLine = #line
let currentFunction = #function

let _ = (greeting(), currentLine, currentFunction)
