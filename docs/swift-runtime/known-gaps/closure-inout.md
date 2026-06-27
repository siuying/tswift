# Closures capturing `inout` — known gaps

Status: implemented for the common case (explicit closure parameters typed
`(inout T) -> …`, called via `f(&x)`), including writeback when the closure
throws. Remaining gaps require type-flow / sema work and are deferred:

1. **`$0` shorthand with `inout`.** A closure like
   `let f: (inout Int) -> Void = { $0 += 1 }` is not recognised as having an
   `inout` parameter, because shorthand closures expose no explicit `Param`
   nodes. Supporting this needs the closure's contextual type signature to be
   propagated to the call site so `has_inout` can be inferred. Until then,
   write the parameter explicitly: `{ (n: inout Int) in n += 1 }`.

2. **Missing `&` / argument validation.** Calling an `inout` closure without
   `&` (or with too few arguments) is not diagnosed at runtime; the parameter
   simply mutates a local copy with no writeback, and missing args default to
   `nil`. This should be rejected — ideally in sema, since it is a static
   error in Swift.

3. **Closure-literal `throws` annotation.** `{ (n: inout Int) throws in … }`
   is not yet parsed (the parser stops at `throws` in the closure signature).
   Throwing closures still work via inference when the annotation is omitted.
