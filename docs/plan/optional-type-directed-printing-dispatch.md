# Type-directed optional printing and dispatch

Fixes GitHub issues **#241** (optional-aware collection printing) and **#242**
(declared-type-aware dispatch for `Optional.take()` / `.debugDescription`).

## Background

The runtime flattens optionals: absent = `SwiftValue::Nil`, present = the
wrapped value itself. By the time a value reaches `print` or method dispatch,
the "this was an optional" fact is erased. Boxing optionals in the value model
was considered and rejected (see #241) — the flattened representation is
load-bearing across promotion, unwrapping, dispatch, hashing, Codable, and
JSON bridging.

The accepted approach is **type-directed**: recover optionality from *static*
type information (written annotations via `TypeRepr`, plus literal shape) and
consult it at the two consumer sites — the describe path and method dispatch.

Constraint discovered during survey: `tswift_sema`'s `Type` enum is
scalar-only (`Int`/`Double`/`String`/`Bool`/`Void`/`Regex`); it cannot express
`Optional<T>` or `[T?]`. The usable static-type source at runtime is the
annotation text parsed by `tswift_ast::TypeRepr`, which already models
optionals, arrays, and dictionaries. No sema changes are required.

## Stage 1 — Foundation: retain declared types on bindings (shared)

1. Extend `Binding` (`crates/tswift-core/src/env.rs`) with
   `declared_type: Option<Rc<str>>`. Type-level metadata only; does not
   participate in equality or mutation semantics.
2. Populate it at binding sites in the interpreter:
   - `var`/`let` with a written annotation → the annotation text verbatim.
   - Function/closure parameters → the parameter's written type.
   - Un-annotated `let x = <literal>` → best-effort synthesis from literal
     shape (Stage 2's inference helper).
3. Add a small `static_type_of(&self, expr: &Node) -> Option<String>` helper
   on the interpreter (likely `interp.rs` or a new `interp/static_type.rs`):
   - Identifier → the binding's `declared_type`.
   - Call to a user function → its declared return type.
   - `as`/`as?` cast → the cast target.
   - Array/dict literal → synthesized from element shape (see Stage 2).
   - Anything else → `None` (graceful degradation: current behavior).

## Stage 2 — Issue #241: optional-aware printing

1. **Literal-shape inference.** An array/dict literal has optional elements
   when any element is a `nil` literal, an `Optional(x)` construction, or an
   identifier whose `declared_type` is optional. Synthesize e.g. `[String?]`
   (element base type may be unknown; only the optionality bit matters, so a
   placeholder like `[T?]` is acceptable — the describe path only calls
   `TypeRepr::is_optional()` on the element repr).
2. **Type-directed describe.** Extend the display seam (`StdContext::display`
   / the `Display` impl helpers in `crates/tswift-core/src/value.rs`) with a
   typed variant: `display_typed(value, Option<&TypeRepr>)`. When rendering a
   collection whose element repr `is_optional()`:
   - `Nil` element → `nil` (unchanged);
   - present element → `Optional(` + *debug* rendering of the element
     (strings quoted/escaped via the existing `fmt_element` rules) + `)`.
   Recurse into nested collections using `TypeRepr::array_element` /
   dictionary key–value reprs.
3. **Wire the call sites.** In the native-call path for `print` /
   `debugPrint`, compute `static_type_of` for each argument expression and
   pass it through (extend `Arg` or `StdContext` — follow whichever is the
   smaller seam). Missing type → exactly today's output.
4. **Non-goals** (document, don't attempt): values whose static type is
   unrecoverable (`Any` collections, elements returned through untyped
   dynamic paths). Top-level `print(x)` for `x: String?` already prints
   `Optional("x")` and must not regress.
5. Tests: `[Optional("x"), nil]`, optional Int/Double/Bool/String elements,
   nested `[[String?]]`, annotated vs. literal-inferred, non-optional arrays
   unchanged, separator/terminator interplay. Update
   `website/src/pages/status/stdlib.mdx` (remove the divergence entry).

## Stage 3 — Issue #242: declared-type-aware dispatch

1. In method dispatch (`crates/tswift-core/src/interp/dispatch.rs`), before
   receiver-kind routing: compute `static_type_of(receiver)`; if it parses as
   optional **and** the member is one `Optional` defines for present values
   (`take`, `debugDescription`), route to the `Optional` receiver intrinsics.
   All other members fall through to today's wrapped-type dispatch, so
   optional chaining (`opt?.count`) is untouched.
2. Register the missing intrinsics in `crates/tswift-std/src/optional.rs`
   (currently intentionally omitted — update the header comment):
   - `take()` — returns the present value (or `Nil`), writes `Nil` back to
     the receiver's storage. Reuse the existing mutating-method write-back
     (`place`) machinery in dispatch.
   - `debugDescription` — `Optional(<debug element>)` for present,
     `"nil"` for absent. Share the element-rendering helper from Stage 2.
3. Tests: `take()` on present/absent optionals (value and write-back),
   `debugDescription` on present/absent, `opt?.count`-style chaining
   regression, `let` receiver for `take()` diagnoses as immutable. Update
   `stdlib.mdx` (deferred note + `Optional<Wrapped>` coverage row).

## Stage 4 — Wrap-up

- `scripts/presubmit` green at each stage boundary.
- Land as (at least) two commits matching the issues:
  `feat(core): optional-aware collection printing` then
  `feat(core): declared-type-aware Optional dispatch`.
  Stage 1 may fold into the first commit or stand alone as
  `refactor(core): retain declared types on bindings`.

## Risks / open questions

- **`Binding` churn**: every `Binding { value, mutable }` construction site
  needs the new field — mechanical but wide; use a constructor to contain it.
- **Seam choice in Stage 2.3**: `Arg` vs. `StdContext` for carrying the type
  hint into native fns — decide by whichever touches fewer signatures.
- **Placeholder element types**: `[T?]`-style synthesized reprs must never be
  used for coercion, only for `is_optional()` checks in describe/dispatch.
- **Interpolation** (`"\([Optional("x")])"`) also diverges in Swift's favor;
  explicitly out of scope here — file a follow-up if desired.
