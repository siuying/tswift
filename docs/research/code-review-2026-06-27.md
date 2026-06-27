# Code Review — 27 commits (gpt-5.5 codex, one reviewer per commit)

## Follow-up status (2026-06-27)

Fixed, each its own commit + pushed to `main`:

1. `fix(std): Bool.toggle returns Void and rejects arguments`
2. `fix(parser): only treat @unknown as a switch clause boundary`
3. `fix(core): support while let and while case condition lists`
4. `fix(core): honor default: in dictionary subscript compound assignment`
5. `fix(core): propagate failing super.init? to subclass initializer`
6. `fix(regex): prevent infinite recursion on nullable quantifiers`
7. `fix(regex): validate counted quantifier ranges and bounds`
8. `fix(wasm): truncate output on UTF-8 char boundaries`
9. `fix(std): classify Character predicates over the whole grapheme`
10. `fix(core): derive Comparable operators and route min/max through the hook`
11. `fix(core): iterate strings by grapheme cluster, matching count`
12. `feat(core): apply tuple return-type labels to returned values`

Deferred (need design work, not quick patches — tracked as enhancements):

- **Array.index(after:)/index(before:)** (`50df73b`) — method intrinsics receive
  positional `Vec<SwiftValue>` with labels already dropped; supporting these
  overloads needs label plumbing through the whole intrinsic signature.
- **Nested-type scoping** (`ed3ec7d`) — nested types are registered globally by
  simple name; correct scoping needs qualified-name registration + an
  enclosing-type lookup table.
- **Switch exhaustiveness by subject type** (`642b05b`) — the check is sound
  (false negatives only). Using the subject's enum type needs a nominal/enum
  variant in sema's `Type` and enum-typed binding tracking. (`.gitignore`
  corruption it mentioned was already fixed in `f8904ab`.)
- **Int→Double literal provenance** (`ac60123`) — runtime promotes any Int in
  mixed arithmetic; matching Swift (only integer *literals* coerce) needs
  literal-ness tracked through sema/interp and a diagnostic for non-literal
  mixes.

---


Date: 2026-06-27. Each commit reviewed in isolation (`<sha>^..<sha>`) by a
separate gpt-5.5 reviewer. Per-commit raw reviews live under `/tmp/reviews/<sha>.md`.

## Verdict summary

| Commit | Area | Verdict |
|--------|------|---------|
| cef3b88 | stdlib S5 Sequence/Collection | Needs-work |
| f767d84 | stdlib S6 Dictionary CoW | **Needs-work (Critical)** |
| 7b16d49 | stdlib S7 Set CoW | Ready-with-fixes |
| de493b5 | stdlib S8 String/graphemes | **Needs-work (Critical)** |
| c1cfa9c | stdlib S9 protocols | **Needs-work (Critical)** |
| 1d6f176 | stdlib S10 containers | Ready-with-fixes |
| 50df73b | Array index helpers | **Needs-work (Critical)** |
| cbd884b | Array.replaceSubrange | Ready-with-fixes |
| 56c15a3 | Array.sort + dict filter | Ready-with-fixes |
| 25247bd | Character predicates | **Needs-work (Critical)** |
| 1f29b16 | Bool intrinsics | Ready-with-fixes |
| 7712f3d | Bool.random | Ready-with-fixes |
| bcf2b4c | Bool shadow guard | Ready-with-fixes |
| 763b3e7 | one-sided ranges / multi-name | Ready-with-fixes |
| e55cee7 | @unknown default | **Needs-work (Critical)** |
| d70c835 | named tuple access | **Needs-work (Critical)** |
| f4e08aa | metatype values | Ready-with-fixes |
| 2a25481 | static subscripts | Ready-with-fixes |
| 84d74a9 | failable init | **Needs-work (Critical)** |
| ed3ec7d | nested types | **Needs-work (Critical)** |
| 888065c | operator refs / if-case | **Needs-work (Critical)** |
| 642b05b | non-exhaustive enum diag | **Needs-work (Critical)** |
| b6d0f40 | dict-element key/value labels | Ready |
| 332187f | regex literals | **Needs-work (Critical)** |
| 893d28e | run sandbox in wasm | **Needs-work (Critical)** |
| c86e0ce | wasm RNG seed fix | Ready-with-fixes |
| ac60123 | preset gaps A/H/K/P | **Needs-work (Critical)** |

## Critical issues (fix first)

1. **f767d84** `interp.rs:2187/2202` — subscript assignment only evaluates the
   first index, so `dict[key, default: 0] += 1` ignores the default → fails
   instead of inserting `1`.
2. **de493b5** `interp.rs:4401` — String sequences materialize via `s.chars()`
   (scalars) while `String.count` reports graphemes; `map`/`filter`/`for-in`/
   `Array("e\u{301}")` operate on scalars. Inconsistent grapheme model.
3. **c1cfa9c** `interp.rs:2695` — only the exact operator symbol dispatches;
   Comparable types defining `static <` don't derive `<=`/`>`/`>=`. Also
   `free.rs:102/127` global `min`/`max` ignore the new Comparable hook.
4. **50df73b** `array.rs:260` — `Array.index(after:)`/`index(before:)`
   unimplementable because labels are dropped before intrinsics, yet
   `Array.index` is claimed as covered.
5. **25247bd** `string.rs:173` — Character predicates classify only the leading
   scalar; `Character("e\u{301}").isASCII` returns `true` (Swift: `false`).
6. **e55cee7** `parser/lib.rs:813` — *any* `@Attribute` now terminates a case
   body, regressing valid attributed decls inside `case` bodies.
7. **d70c835** `interp.rs:1378/1413/4737` — tuple labels only come from literals;
   function return-type labels ignored, so `f().lo` for `-> (lo:Int,hi:Int)`
   doesn't work (the promised semantics).
8. **84d74a9** `interp.rs:4258` — `super.init()` via `run_class_init` collapses
   `Return(nil)` to success; a failing superclass `init?` yields an instance
   instead of `nil`. Also failable-kind not stored in AST (plain `init { return nil }` accepted).
9. **ed3ec7d** `interp.rs:766/4154` — nested types registered globally by simple
   name → cross-type collisions, scope leakage, and `Outer.Nested` not verified
   to belong to `Outer`. (Also `.gitignore` corrupted to `targetloop-log.md`.)
10. **888065c** `interp.rs:3231` — `while case`/`while let` lowered to a
    condition list but `eval_while` only evaluates the first child → "unsupported
    construct: CaseCondition".
11. **642b05b** `sema/lib.rs:353` — exhaustiveness infers the enum from
    *referenced case names*, not the switch subject type → false negatives across
    enums sharing case names.
12. **332187f** `regex.rs:548/668` — greedy unbounded repeat over nullable
    subpattern (`/()*/`, `/(a?)*/`) recurses forever / stack-overflow. Also
    `{3,2}` and huge counts unguarded.
13. **893d28e** `tswift-wasm/lib.rs:70` — `truncate` slices by byte index; a
    multibyte UTF-8 boundary panics/traps wasm. Plus per-run `Box::leak` of
    `Analysis` leaks memory each browser run.
14. **ac60123** `sema/lib.rs:471`, `ops.rs:19` — Int→Double coercion applied by
    value/type, not literal provenance; `let i = 5; let d: Double = i` and
    `print(d + i)` wrongly compile/run.

## Recurring themes

- **Argument arity/validation silently ignored**: `Bool.toggle(x)`, `Bool.random(1)`,
  string prefix/suffix negative counts, sequence `dropFirst(-1)` — should `Trap`/`Type`-error.
- **Grapheme vs scalar** model is inconsistent between `String.count` and sequence ops.
- **RNG determinism**: `randomElement`/`shuffled` keyed off length only (cef3b88);
  fixed wasm seed makes `Bool.random()` deterministic (c86e0ce).
- **`.key`/`.value` on any 2-tuple** — flagged in 56c15a3 & d70c835, **fixed in b6d0f40** (verify others land on the fix).
- **Checklist/inventory marked complete ahead of implementation**: 1d6f176 (Result,
  ArraySlice), f4e08aa (.Type/.Protocol), 642b05b (exhaustiveness), 50df73b (Array.index).

## Clean

- **b6d0f40** — Ready, no issues (correctly removes the over-permissive
  `.key`/`.value` fallback and adds +/- tests).
