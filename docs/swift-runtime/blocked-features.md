# Blocked / Partial Features

Features that cannot be completed without a larger frontend change or a design
decision needing human input. Each entry records what works, what is missing,
and why it is blocked.

## Named tuple element access (`r.min` on `-> (min: Int, max: Int)`)

**Status:** positional access (`r.0`, `r.1`) works; named element access does not.

**What's missing**
- The parser rejects labeled tuple literals `(min: 1, max: 9)`
  (`expected RParen, found Colon`).
- Sema does not propagate a function's tuple return type to the call site:
  `let r = f()` types `r` as `:Void`, so the `MemberExpr "min"` carries no
  resolved element index.
- `SwiftValue::Tuple` stores positional values only, with no element labels.

**Why blocked**
Supporting `r.min` end-to-end requires (a) parsing labeled tuple types/literals,
(b) sema carrying tuple element labels into the resolved type of expressions,
and (c) the runtime `SwiftValue::Tuple` carrying labels (a value-representation
change touching every tuple match site). This is a multi-crate change that
should be designed deliberately rather than bolted on. Positional multiple-return
covers the core "multiple return values via tuples" capability today.
