// A result builder must be a struct or class — the interpreter cannot dispatch
// static methods on an enum/actor builder.
@resultBuilder
enum EB { // expected-error{{must be a 'struct' or 'class'}}
    static func buildBlock(_ parts: String...) -> String { parts.joined() }
}
