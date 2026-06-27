# Extensions on builtin types — known gaps

Status: `extension Int`, `extension String`, `extension Array`, etc. add
methods and computed properties dispatched on the value-typed receiver,
including `mutating` methods (the updated receiver is written back through the
caller's storage, even on a thrown error) and unqualified self-property reads
(`count`, `isEmpty`) inside those methods. Constrained (`where`) extensions add
their members too.

## Gaps

1. **`where` clauses are not enforced.** `extension Array where Element: Numeric`
   makes its members visible on *every* array, not only numeric ones. This
   matches the interpreter's general policy of skipping generic/where
   constraints at runtime (no static type checking); valid programs only call
   such members on conforming element types.

2. **Mutating a non-lvalue receiver is not diagnosed.** `5.double()` or
   `makeInt().double()` (a literal / temporary) silently runs the method and
   discards the mutation instead of being rejected as a non-lvalue, because no
   write-back `place` resolves. Write-back through a subscript receiver
   (`arr[0].double()`) is likewise not supported.

3. **Dispatch order.** Builtin stdlib intrinsics and sequence algorithms are
   consulted before user extension methods, so an extension method cannot
   shadow a real stdlib member of the same name. This is acceptable for
   otherwise-invalid redeclarations.
