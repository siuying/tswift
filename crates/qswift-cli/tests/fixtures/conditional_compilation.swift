#if os(macOS)
let system = "macOS"
#elseif os(Linux)
let system = "Linux"
#else
let system = "unknown"
#endif
func currentLine() -> Int { return #line }
print(currentLine() > 0)
print(system == "macOS" || system == "Linux" || system == "unknown")
