# Explicit generic type specialization — notes & gaps

Status: `Type<Args>(…)`, `Type<Args>()`, and `Type<Args>.member` now parse and
run for generic struct/class/enum. The parser recognises an angle group that is
balanced, contains only type-like tokens, and is immediately followed by `(`
(same line) or `.`; otherwise `<`/`>` keep their comparison meaning. The runtime
infers concrete type arguments from values, so the `<…>` clause is discarded
after parsing. Extensions on generic types and constrained (`where`) extensions
work because they extend the type by its base name.

## Gaps

1. **`some` / `any` as enum case or member names.** These are contextual
   keywords; an enum `case some(T)` (as in a hand-rolled `Optional`-like type)
   fails to parse. Use a non-keyword case name. Unrelated to specialization.

2. **No type-argument checking.** Because the `<…>` clause is discarded, a
   mismatched explicit argument (`Stack<String>(items: [1, 2])`) is not
   diagnosed; the runtime uses the value types.

3. **Bare generic comparison calls.** A genuine `a<b>(c)` expression intended as
   `(a < b) > (c)` will be parsed as a specialization when `b`/`c` are
   identifier-only. This matches Swift's own parsing heuristic. Comparison and
   ternary forms with multi-char or logical operators (`a < b && b > (c)`,
   `a < b ? c > (d) : e`, `(x ?? y) > (z)`) are *not* swallowed — the scan only
   accepts pure angle operators and type-list tokens between the brackets.

4. **Optionals / compositions in explicit arguments.** Because the interior
   scan accepts only names, `.`, `,`, `[ ]`, and `:`, an explicit argument that
   spells an optional (`Stack<Int?>(…)`) or a protocol composition
   (`Box<A & B>(…)`) is not recognised as a specialization. Drop the explicit
   clause and let inference supply the type, which is the common form.
