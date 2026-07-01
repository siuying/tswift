// Macro declarations are accepted and ignored (no expansion engine), and
// @convention type attributes parse in annotation position.

@freestanding(expression)
macro stringify<T>(_ value: T) -> (T, String) =
    #externalMacro(module: "MacrosPlugin", type: "StringifyMacro")

@attached(member)
macro AddInit() = #externalMacro(module: "MacrosPlugin", type: "AddInitMacro")

let cFn: @convention(c) (Int) -> Int = { $0 * 2 }
print(cFn(21))
let blockFn: @convention(block) () -> String = { "blk" }
print(blockFn())

// `macro` stays usable as an ordinary identifier.
var macro = 10
macro += 5
print(macro)
