# `ExpressibleBy*Literal` — known gaps

Status: a binding annotated with a user struct/class that conforms to
`ExpressibleByArrayLiteral`, `ExpressibleByStringLiteral`,
`ExpressibleByIntegerLiteral`, `ExpressibleByFloatLiteral`,
`ExpressibleByBooleanLiteral`, or `ExpressibleByNilLiteral` is built through the
type's literal initializer when the right-hand side is the matching *literal
syntax* (not an arbitrary expression). An optional annotation (`T?`) keeps a
`nil` literal as the absent optional.

## Gaps

1. **One initializer per struct.** A struct stores a single `init`, so a type
   declaring several literal initializers (e.g. both `init(nilLiteral:)` and
   `init(stringLiteral:)`) only keeps the last; the others are not selectable.
   This is a pre-existing struct-init-overloading limitation. Types with a
   single literal initializer work. (Classes have the same single-`init` limit.)

2. **No initializer shape checking.** The literal initializer is invoked
   positionally without verifying its parameter shape, so a malformed
   `init(arrayLiteral:)` is not diagnosed.

3. **Shallow element conversion.** Array-literal elements are passed through as
   evaluated values; they are not themselves recursively converted to a nested
   `ExpressibleBy*Literal` element type.

4. **Only `let`/`var` annotations.** Conversion is driven by the declaration's
   type annotation. Literal conversion in other contextual positions (function
   arguments, returns) is not yet applied.

5. **Dictionary literals.** `ExpressibleByDictionaryLiteral` is not handled.

6. **Negated numeric literals.** `-5` parses as a prefix expression over a
   literal, so it is not treated as integer-literal syntax for conversion.
