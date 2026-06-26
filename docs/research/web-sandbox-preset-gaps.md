# Web-sandbox preset gaps

Date: 2026-06-26
Scope: `prototype/web-sandbox/src/pages/index.astro` preset examples vs. the
pure-Rust frontend + interpreter (`qswift-cli run`, same crates the wasm wraps).

Each preset was run through `target/debug/qswift run`. This document records
**every** failure, its root cause (reduced to a minimal repro), the path to
support it, and a **gap score `n/10`** = relative effort / number of steps to
land it (0 = trivial preset-text fix, 10 = deep cross-cutting work).

## Status summary

| Preset | Runs? | Blocking gap(s) | Gap |
|--------|-------|-----------------|-----|
| Hello World | ✅ | — | — |
| Fibonacci | ✅ | ~~tuple-destructuring assignment `K`~~ (fixed; preset restored to the tuple-swap form) | 5 |
| Classes | ✅ | — | — |
| Strings | ✅ | — | — |
| Generics | ✅ **fixed** | ~~`J` preset escaping bug~~ | 0 |
| Collections | ✅ **fixed** | ~~`I` `Array.sort()`, `L` dict element `.key`/`.value`~~ | 3 |
| Switch Patterns | ✅ **fixed** | ~~`B` one-sided range patterns~~ | 3 |
| Protocols | ✅ **fixed** | ~~`B` one-sided range patterns~~; ~~`P` protocol default-method dispatch via a composition typealias~~ | 4 |
| Closures & HOF | ✅ **fixed** | ~~`C` operator function references~~ | 4 |
| Error Handling | ✅ **fixed** | ~~`F` `Character.isLetter/isNumber`, `G` `if case` binding~~ | 4 |
| Structs | ✅ **fixed** | ~~`D` multi-name binding~~, ~~`A` Int→Double coercion~~ | 8 |
| Enums | ✅ **fixed** | ~~`A` Int→Double coercion~~ | 8 |
| Optionals | ✅ **fixed** | ~~`H` array `as [T?]` cast~~, ~~`A` coercion~~ | 8 |

**All 13 presets now run.** Every documented gap is closed; the `wasm_smoke`
suite executes all presets through the compiled wasm artifact.

### Landed (gap < 4)

All gaps scored `< 4` are fixed and verified end-to-end. Each ships a golden
fixture and a `qswift-cli` run fixture; `cargo test` is green and the
`wasm_smoke` suite runs the re-enabled presets through the compiled wasm.

- **`J` Generics escaping** — `prototype/web-sandbox/src/pages/index.astro` now
  doubles the `\(` interpolation markers (the nested `\"` already collapse to
  plain `"` under the JS template literal). `supported: false` removed.
- **`I` `Array.sort()` / `sort(by:)`** — mutating intrinsic in
  `crates/qswift-std/src/array.rs` delegating to the shared `sorted` algorithm;
  registry key `Array.sort`.
- **`L` dict element `.key` / `.value`** — named-tuple member access in
  `crates/qswift-core/src/interp.rs`, plus `Dictionary.filter` returning a
  `Dictionary` (registry key `Dictionary.filter`) so `scores.filter{…}.keys`
  chains.
- **`B` one-sided range patterns** — parser accepts `case n...:`, `case ..<n:`,
  `case ...n:` (single-bound `RangePattern` tagged `from`/`upTo`/`through`);
  matcher in `interp.rs` handles them. Unblocked **Switch Patterns**; also closed
  the range-pattern half of **Protocols**.
- **`D` multi-name binding** — `var a, b, c: T` / `let x = 1, y = 2` desugar to
  N bindings in `crates/qswift-parser/src/lib.rs` (shared annotation deep-copied
  via `Ast::clone_subtree`). Closed the structural half of **Structs**.

Proof fixtures:
`crates/qswift-cli/tests/fixtures/{array_sort,dict_element_members,one_sided_range_patterns,multi_name_binding}.{swift,expected}`,
`tests/swift-fixtures/tier1-imperative/one_sided_range_patterns.swift`,
`tests/swift-fixtures/tier2-value-types/multi_name_binding.swift`, and additions
to `tests/swift-fixtures/tier10-stdlib/{s4-array,s6-dictionary}.swift`.

> **Note on Protocols:** removing gap `B` revealed a second gap (`P`): a
> protocol's default method was not dispatched for a struct conforming via a
> **protocol-composition typealias**. Fixed under gap `P` below; the Protocols
> preset now runs.

### Landed (gap = 4)

Both gap-`4` presets are fixed and verified end-to-end (golden + `qswift-cli`
fixtures, full `cargo test` green, `wasm_smoke` runs both re-enabled presets).

- **`C` operator function references** — a bare operator in value position
  (`reduce(0, +)`, `sorted(by: >)`) now resolves to a synthesized operator
  closure. `ClosureDef` became an enum (`User` / `Operator(op)`) in
  `crates/qswift-core/src/interp.rs`; `eval_ident` returns a `Closure` for an
  operator token (`is_operator_name`) and `call_closure` applies the operator
  via `ops::binary`/`ops::unary`, so the stdlib HOFs call it like any closure.
  Unblocked **Closures & HOF**.
- **`F` Character predicates** — `isLetter`/`isNumber`/`isWhitespace`/
  `isUppercase`/`isLowercase`/`isNewline`/`isWholeNumber`/`isHexDigit`/`isASCII`
  registered as properties on the (single-grapheme) `String`/`Character`
  receiver in `crates/qswift-std/src/string.rs`, classifying the leading
  Unicode scalar via `char` methods (no new deps, offline-safe).
- **`G` `if case` / `guard case` binding** — a refutable `case` pattern in a
  condition now lowers to a new `CaseCondition` runtime node
  (`crates/qswift-frontend/src/{kind,compat}.rs`) instead of an
  `OptionalBinding`; `eval_cond_list` evaluates the subject, runs the existing
  `match_pattern`, and binds payloads on success. Together `F`+`G` unblocked
  **Error Handling**.

Proof fixtures:
`crates/qswift-cli/tests/fixtures/{operator_references,character_predicates,if_case_binding}.{swift,expected}`,
plus additions to `tests/swift-fixtures/tier3-reference-arc/closures.swift` and
`tests/swift-fixtures/tier5-errors-modules/error_handling.swift`.

---

## Gap inventory (ranked easiest → hardest)

### J. Generics preset — interpolation escaping bug — `0/10` — ✅ FIXED
**Not a runtime gap.** The Generics preset is the only one written with
single-backslash `\(…)` interpolation inside the JS template literal
(`index.astro` lines ~335–339). JS collapses `\(` → `(`, so the emitted Swift
loses its interpolation markers and `print("…: \(largest([\"banana\", …])!)")`
becomes invalid (`"…: (largest(["banana", …` breaks the string).

Repro of the *correct* Swift (all features already supported): runs and prints
`largest int: 9`, `largest str: cherry`, swap, etc.

**Path:** double the backslashes (`\(` → `\\(`, `\"` → `\\"`) like every other
preset, then drop `supported: false`. No frontend work.

---

### I + L. Collections — `Array.sort()` and dict element members — `3/10` — ✅ FIXED
Two independent gaps (plus `Dictionary.filter` returning a `Dictionary`):

- **`I` `Array.sort()` (in-place mutating).** `var a=[3,1,2]; a.sort()` →
  `unsupported construct: method .sort() on Array`. Note `a.sorted()` (the
  non-mutating form) already works (`qswift-std/src/sequence.rs:27`).
- **`L` dictionary element `.key` / `.value`.** `s.filter { $0.value >= 90 }`
  → `member .value on tuple`. Dictionary iteration yields a labelled
  `(key:, value:)` tuple, but member access by label on a tuple value isn't
  wired.

**Path:**
- `sort()`: add a mutating sibling to `sorted` in `qswift-std/src/sequence.rs`
  that writes back through the receiver lvalue (the runtime already has
  mutating-method plumbing for structs/arrays — e.g. `append`).
- `.key`/`.value`: support named-tuple member access in the interpreter
  (`qswift-core/src/interp.rs` member-access path) for the dictionary element
  tuple shape.

Both are localized; `mapValues`, `flatMap`, full `Set` algebra already work.

---

### B. Switch / Protocols — one-sided range patterns — `3/10` — ✅ FIXED
`case 90...:` and `case ..<0:` fail at parse:
`expected an expression, found Colon`. The two-sided form `case 80..<90:`
already works, so only the open-ended prefix/postfix range *pattern* is missing.
This single gap blocks **two** presets (Switch Patterns directly; Protocols via
the `grade()` extension's `switch score`).

**Path:** in the parser (`qswift-parser/src/lib.rs`) accept a one-sided range
expression in pattern position (`expr...`, `...expr`, `..<expr`), lowering to
the existing `PartialRangeFrom`/`PartialRangeUpTo` pattern the runtime can match
with `~=`. Sema/runtime already model the range types.

---

### D. Structs (part 1) — multi-name binding `var a, b, c, d: T` — `3/10` — ✅ FIXED
`struct M { var a, b, c, d: Double }` →
`consecutive statements on a line must be separated by ';'`. The parser doesn't
accept a comma-separated name list sharing one type annotation.

**Path:** in declaration parsing, after a binding name accept `, name…` and
desugar to N separate stored properties with the shared type annotation. Pure
parser/AST work. (Captured historically as `structs_multi_binding_gap.swift`.)

---

### C. Closures & HOF — operator function references — `4/10` — ✅ FIXED
`numbers.reduce(0, +)` → `unknown variable: +`. Passing a bare operator as a
function value isn't resolved. The closure form `reduce(0) { $0 + $1 }` works.

**Path:** resolve operator tokens used in value position to a builtin
function value. Touches name resolution in sema + a small set of operator
thunks in the runtime (`qswift-core/src/ops.rs` already centralizes operator
semantics, so the thunks can delegate there).

---

### F + G. Error Handling — Character predicates and `if case` binding — `4/10` — ✅ FIXED
Two gaps:

- **`F` `Character.isLetter` / `.isNumber`** (and friends). A `Character` is
  modelled as a single-grapheme `String` (`qswift-std/src/string.rs:12`) but the
  `isLetter`/`isNumber`/`isWhitespace`/… predicate properties aren't registered
  → `member .isLetter on String`.
- **`G` `if case .success(let v) = r`** → `binding without a name`. The
  equivalent `switch` case binding (`case .success(let v):`) works, so the
  pattern machinery exists; it just isn't reused by `if case` / `guard case`.

**Path:**
- Add the Character predicate intrinsics in `qswift-std/src/string.rs` (Unicode
  scalar classification on the single grapheme).
- Route `if case` / `guard case` through the same enum-pattern binding path the
  `switch` arm already uses (`qswift-core/src/interp.rs`).

`throws`/`do`-`catch`, `catch E.t(let n)`, `try?`, `Result`, `defer` all work.

---

### K. Fibonacci (underlying) — tuple-destructuring assignment — `5/10` — ✅ FIXED
`(a, b) = (b, a + b)` (and `var (a, b) = (0, 1)`) →
`consecutive statements on a line must be separated by ';'`. The preset was
reworked to the validated iterative form, so it *runs today*, but the language
gap remains.

> **Landed.** Root cause was the parser gluing a newline-separated `(` into a
> postfix call (`1\n(a, b)` → `1(a, b)`); a call argument list now must begin on
> the callee's line (`qswift-parser`). The runtime then handles a `TupleExpr`
> assignment target: it evaluates the whole RHS first (so swaps are correct) and
> writes each element back through its lvalue (`assign_destructured` in
> `qswift-core/src/interp.rs`), supporting nested tuples and `_` discards. The
> preset's iterative variant was restored to the idiomatic
> `(a, b) = (b, a + b)` form.

**Path:** parser must accept a tuple expression as an assignment lvalue; sema
must check arity/types element-wise; runtime must evaluate the RHS tuple fully
before binding (so swaps are correct) and write each element back through its
lvalue. Three layers, but bounded.

---

### H. Optionals (part 1) — array cast to optional element `as [T?]` — `5/10` — ✅ FIXED
`[Contact?]` via `… as [Contact?] + [nil]` →
`could not cast value to [Contact?]`. Casting `[T]` to `[T?]` (covariant
element wrap) isn't handled by the runtime cast.

> **Landed.** `value_is_type` now recognises array target types: every element
> must match the element type, and an optional element (`[T?]`) also accepts
> `nil`. Because the runtime models an optional value as the bare value (or
> `Nil`), no per-element wrapping is needed — the array passes through unchanged
> (`qswift-core/src/interp.rs`, helper `array_element_type`).

**Path:** in the runtime `as`/cast logic (`qswift-core/src/interp.rs` +
`value.rs`), recognize array casts where the target element is the optional of
the source element and wrap each element in `.some`. `guard let`, optional
chaining, `??`, `compactMap`, `try?` already work.

---

### A. Structs/Enums/Optionals (core) — integer-literal → Double coercion — `8/10` — ✅ FIXED
The single highest-impact gap. Swift implicitly converts integer literals to
`Double` when the context demands it; the runtime keeps them `Int` and then
fails on mixed arithmetic. Repros:

- `let r: Double = 5` → analysis error `cannot convert 'Int' to 'Double'`
- `f(r: 5)` where `f(r: Double)` → `operator * cannot apply to Int and Double`
- `Point(x: 3, y: 4)`, `radius: 5`, `var x = 0.0; x += 1` → same family

This blocks **Structs** (`Point(x:3,y:4)`, `translate(dx:10,dy:0)`), **Enums**
(`radius: 5`), and contributes to **Optionals**.

**Path:** implement numeric-literal type inference: an integer literal in a
`Double` (or other `ExpressibleByFloatLiteral`/`ExpressibleByIntegerLiteral`)
context must be typed/stored as the contextual type. This needs:
1. Sema: propagate the contextual/expected type into literal expressions
   (parameter types, declared `let`/`var` annotations, struct field types,
   binary-op operand unification).
2. AST/lowering: carry the resolved literal type to the runtime.
3. Runtime: construct the literal as `Double` (not `Int`) when so typed; or a
   uniform numeric-promotion rule in `qswift-core/src/ops.rs`.

High effort because it touches the type-inference contract end-to-end, not a
single intrinsic. Worth doing first conceptually — it unblocks the most
realistic programs — but it is the largest change.

> **Landed (pragmatic two-layer rule).** Rather than full contextual literal
> typing, the runtime coerces an integer at the typed boundaries and promotes
> mixed arithmetic:
>
> 1. **Sema** (`qswift-sema`) treats `Int → Double` as coercible, so an
>    annotated binding (`let r: Double = 5`) and mixed arithmetic (`d / 4`) no
>    longer diagnose; mixed binary ops infer `Double`.
> 2. **Runtime ops** (`qswift-core/src/ops.rs`) promote the integer side of a
>    mixed `Int`/`Double` binary op to `Double`.
> 3. **Runtime coercion** (`coerce_numeric`) converts an integer literal to
>    `Double`/`Float` at annotated `let`/`var` bindings, struct memberwise
>    fields (`StoredProp.ty`), enum associated values
>    (`EnumCaseDef.payload_types`), and function parameters (`Param.ty`).
>
> Pure integer code is unaffected (integer `/` and `%` keep integer semantics).
> This is a documented fidelity tradeoff: the runtime promotes any mixed
> `Int`/`Double` arithmetic, whereas Swift only coerces integer *literals*.

---

### P. Protocols — default-method dispatch via a composition typealias — `4/10` — ✅ FIXED
Uncovered after fixing `B`: a struct conforming through a protocol-composition
typealias (`typealias NamedAndScored = Named & Scorable`; `struct Student:
NamedAndScored`) failed with `method .grade() on Student`, because the runtime
recorded conformance to the *alias* name and never expanded it to its component
protocols, so the default `grade()` from `Scorable` was not found.

> **Landed.** The interpreter now registers protocol-composition typealiases
> (`register_typealias` → `protocol_aliases`) and `all_protocols` expands an
> alias to its components during default-implementation lookup
> (`qswift-core/src/interp.rs`).

---

## Recommended order

1. **Generics escaping fix** — `0/10`, re-enables a preset immediately.
2. **One-sided range patterns `B`** — `3/10`, unblocks Switch + Protocols.
3. **`Array.sort()` + dict `.key/.value` `I`/`L`** — `3/10`, unblocks Collections.
4. **Multi-name binding `D`** — `3/10` (half of Structs).
5. **Operator refs `C`**, **Character predicates + `if case` `F`/`G`** — `4/10`.
6. **Tuple assignment `K`**, **array optional cast `H`** — `5/10`.
7. **Integer-literal → Double coercion `A`** — `8/10`, foundational; fully
   unblocks Structs/Enums/Optionals.

Every fix should land with a golden fixture under `tests/swift-fixtures/` and,
once green, the corresponding preset's `supported: false` flag removed in
`prototype/web-sandbox/src/pages/index.astro`.

## Status: complete

All gaps `A`–`P` are closed and every preset's `supported: false` flag has been
removed. The `wasm_smoke` suite runs all 13 presets through the compiled wasm
artifact; the golden corpus and `qswift-cli` run fixtures cover each fix.
