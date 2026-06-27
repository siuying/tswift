# Plan — Replace `msf` (C) with a Rust Swift Frontend

**Status:** proposed
**Author:** quick-swift
**Date:** 2026-06-25
**Supersedes the FFI sections of:** `docs/plan/swift-runtime-implementation-plan.md` §1.2, §3.1
**Reads-with:**
- `docs/research/2026-06-24-msf-swift-frontend.md` — how msf works (study target)
- `docs/swift-runtime/feature-checklist.md` — full Swift 6.3 surface to cover
- `vendor/msf/` — the C reference frontend (study, **do not copy code**)

---

## 1. Goal

Replace the vendored C library **msf** (lexer → parser → 3-pass sema, reached via
`bindgen`/`cc` FFI in `crates/msf-sys`) with our **own Swift frontend written in safe
Rust**. After this work:

- `qswift` builds with **no C toolchain, no `cc`, no `bindgen`, no `make`** — pure
  Rust, trivially cross-compilable (incl. `wasm32`) and `cargo`-cacheable.
- The frontend is **debuggable, Miri-clean, and editable in one language**.
- We control the AST and diagnostics end-to-end (fix "FE gap" rows in the checklist
  ourselves instead of waiting on upstream C).

**Constraint:** we **study** msf (architecture, token kinds, AST shapes, diagnostic
wording, the SWAR/3-pass designs) but **write original Rust**. No translation of msf
`.c` source, no lifting of its tables verbatim where avoidable. msf stays in-tree only
as a **reference oracle** for differential testing (§6), then is removed (§8).

**Non-goals:** changing runtime behaviour, the `SwiftValue` model, or the stdlib. The
runtime (`tswift-core`/`-std`) should compile **unchanged** against the new
frontend (§3 makes that the hard contract).

---

## 2. The contract: the frontend crate's API is the seam

The runtime never touches msf's C ABI. Today it depends only on the **safe `msf` crate**
(`crates/msf`). That crate's public surface is the entire contract our Rust frontend
must satisfy. As part of this work the crate is **renamed `tswift-frontend`**
(matching the `quick-swift-*` workspace convention) so nothing in the tree is called
`msf` once the C library is gone — no name collision with the thing we replaced.

> **Rename impact:** the runtime's `use msf::{Analysis, Node, NodeKind}` becomes
> `use tswift_frontend::{Analysis, Node, NodeKind}` — a single mechanical
> find-replace across `tswift-core`/`-std`, landed in Step 0 while the C backend is
> still live (so it is verified green before the engine swap). The *types and their
> methods are unchanged*; only the crate path moves.

The contract surface (unchanged by the rename):

| API element | Source | Must reproduce |
|---|---|---|
| `Analysis::analyze(src, file) -> Result<Analysis, AnalyzeError>` | `crates/msf/src/lib.rs:29` | entry point |
| `Analysis::root() -> Node` | `:42` | typed AST root |
| `Analysis::diagnostics() -> Vec<Diagnostic>` | `:53` | errors w/ message + byte range |
| `Analysis::is_ok()` | `:76` | no-error predicate |
| `Node::kind() -> NodeKind` | `:391` | 66 named kinds + `Other(u16)` |
| `Node::children() -> Children` | `:399` | first-child/next-sibling walk |
| `Node::int/float/bool/text()` | `:409–442` | literal payloads |
| `Node::op_text/decl_name/type_name/arg_label` | `:154–352` | name/op payloads |
| `Node::line/jump_label/loop_label/ownership` | `:177–223` | locations + labels |
| `Node::case_info/param_info/var_accessors/modifiers` | `:245–323` | structured sub-views |
| `Node::dump() -> String` | `:372` | s-expr/text dump (for snapshots) |
| `Diagnostic { message, line, col, start, end }` | `:535` | diagnostics |
| `NodeKind` enum (66 variants + `Other`) | `crates/msf/src/kind.rs` | AST vocabulary |

**Strategy:** keep this crate as the **stable public façade** (renamed
`tswift-frontend`), and swap its *backing*. Today it wraps `msf_sys` raw pointers.
After this work it drives the pure-Rust pipeline (§3) over an owned Rust AST. The runtime
sees only `Analysis`/`Node`/`NodeKind`, whose shapes never change.

### 2.1 Kill the raw-discriminant dependency first (prerequisite)

`tswift-core/src/interp.rs:351–354` matches **raw msf discriminants** —
`NodeKind::Other(28)` (operator decl), `Other(27)` (precedencegroup), `Other(11)`
(typealias), `Other(10)` (import). These leak msf's C enum numbering into the runtime
and would pin us to msf's value assignments.

**Step 0 of the migration:** promote every `Other(n)` the runtime matches into a
**named `NodeKind` variant** (`OperatorDecl`, `PrecedenceGroupDecl`, `TypeAliasDecl`,
`ImportDecl`, …). Do this *while C msf is still the backend* so the change is verified
green against the oracle before we swap engines. After this, `NodeKind`'s meaning is
defined by us, not by msf's integers — and our Rust frontend is free to number
internally however it likes.

---

## 3. Architecture of the new frontend

Replace `msf-sys` (FFI) with native Rust crates; rename `crates/msf` →
`tswift-frontend` and re-point it at the pure-Rust pipeline.

```
            BEFORE                                AFTER
  ┌─────────────────────────┐         ┌──────────────────────────────┐
  │ msf-sys  (cc+bindgen,    │         │ tswift-lexer  ─┐              │
  │          C .a, unsafe)   │         │ tswift-ast    ─┼─ pure Rust   │
  └───────────▲─────────────┘         │ tswift-parser ─┤   (safe)     │
              │                        │ tswift-sema   ─┘              │
  ┌───────────┴─────────────┐         └──────────────▲──────────────┘
  │ msf  (safe wrapper)      │  ===>   ┌──────────────┴──────────────┐
  │  Analysis/Node/NodeKind  │         │ tswift-frontend         │
  └───────────▲─────────────┘         │  (same API, drives pipeline) │
              │   (use msf::…)         └──────────────▲──────────────┘
  ┌───────────┴─────────────┐               (use tswift_frontend::…)
  │ tswift-core / -std  │         ┌──────────────┴──────────────┐
  └─────────────────────────┘         │ tswift-core / -std      │
                                       └──────────────────────────────┘
```

### 3.1 New crates

| Crate | Responsibility | Study in msf |
|---|---|---|
| `tswift-lexer` | UTF-8 → token stream; zero-copy spans; NFC for idents; string interpolation spans; raw/multiline/regex delimiters; trivia (comments) | `src/lexer/**`, `src/unicode/**` |
| `tswift-ast` | the AST + `NodeKind` (the *owned* node arena, `TypeRef`, spans). The public `Node`/`NodeKind`/`Type` map onto this. | `src/ast/**`, `generated/` kind tables |
| `tswift-parser` | recursive-descent decls/stmts + Pratt expressions; operator precedence; produces `tswift-ast`; recovers + emits parse diagnostics | `src/parser/**` |
| `tswift-sema` | name resolution + type inference + conformance/witness tables; 3-pass (declare → resolve → conform); emits sema diagnostics | `src/semantic/**`, `src/type/**` |
| `tswift-frontend` | **the consumer-facing crate** (renamed from `msf`): same public API (`Analysis`/`Node`/`NodeKind`/`Diagnostic`); drives the pipeline, owns the resulting `Tree { ast, types, diagnostics }`, and the multi-file module driver | `include/msf.h`, `src/msf.c` |

> Naming: the old `msf` safe-wrapper crate and the throwaway `swift-frontend` façade
> idea are **merged into one** crate, `tswift-frontend`. Internal pipeline crates
> stay `swift-*` (engine), the public crate carries the `quick-swift-*` prefix (product).
> This removes every `msf` name from the workspace once the C lib is decommissioned (§8).

### 3.2 AST representation

Owned arena (`Vec<NodeData>` + indices), not C pointers — gives us `Send`, easy
snapshotting, and Miri-clean borrowing. `Node<'a>` becomes `{ tree: &'a Tree, id:
NodeId }`; the lifetime story stays identical to today (`Node` borrows the analysis),
so the runtime's borrow patterns are unaffected.

- **String interning** (FNV-1a/`ahash` over NFC bytes) for identifiers → `Symbol(u32)`,
  matching msf's pointer-identity-by-intern design.
- **Builtin `TypeRef` singletons** (`Int`, `String`, `Bool`, …) compared by id, as msf
  compares builtins by pointer.
- **Spans** are `(start, end)` byte offsets into the original source (zero-copy), so
  `Diagnostic` byte ranges and `Node::line/col` come for free.

### 3.3 Things we deliberately do **not** port

- The **SWAR/SIMD lexer fast-paths** (`scan/fast.c`) — premature for a correctness
  rewrite. Write a clean, branch-predictable scalar lexer; revisit perf later with our
  own `memchr`/SIMD if profiling demands. *Get it correct, then fast.*
- The **baked SDK vocabulary** (`generated/sdk_vocab*.h`, 187K lines). The runtime
  resolves stdlib *behaviour* itself and only needs type *shapes* for the surface it
  implements. Define a **small Rust-native vocabulary** of the stdlib types the
  checklist's Tier 10 subset touches, generated from a concise table we own — not the
  Apple SDK snapshot. (Track coverage gaps as fixtures.)
- `src/project.c` (Xcode/SwiftPM discovery, POSIX-only) — out of scope; the CLI already
  takes file lists.

---

## 4. Migration milestones (F-series, frontend)

Each milestone keeps the workspace **green** by running the *existing* runtime fixtures
(`crates/tswift-cli/tests/fixtures`, 95 pairs) **and** the new frontend golden
fixtures (§5) against the new engine, diffed vs the C oracle (§6) where it still exists.

**F0 — Scaffold + de-risk the seam (the contract test).**
- Do **Step 0** (§2.1): promote raw-discriminant matches to named `NodeKind`. Verify
  green against C msf.
- Rename `crates/msf` → `tswift-frontend` and update the two `use msf::…` sites in
  `tswift-core`/`-std`. Create `tswift-lexer`, `tswift-ast`, `tswift-parser`,
  `tswift-sema`.
- Stand up the **dual-backend switch**: `tswift-frontend` compiles with either
  `feature="c-oracle"` (old FFI) or `feature="rust"` (new), defaulting to C until F-end.
- Stand up the **golden-fixture harness** (§5) and the **differential harness** (§6).
- *Exit:* lexer + parser + sema produce an AST for `print("hi")`;
  `tswift-frontend`'s `Node`/`NodeKind`/`diagnostics` API returns identical results
  to C msf for it.

**F1 — Tier 0 lexical + Tier 1a (the foundation).** All literals (incl. raw/multiline/
extended-delimiter/regex spans), comments/trivia, operators, ranges, identity, NFC
identifiers; `let`/`var` + type annotations + inference; arithmetic/compound assignment;
tuples + decomposition; wildcard; int conversions.
*Exit:* every Tier 0/1a fixture parses + types identically to the oracle; `arithmetic`,
`bitwise`, `strings` runtime fixtures pass on the Rust backend.

**F2 — Tier 1b/1c (functions + control flow).** Params/labels/defaults/variadics/`inout`,
nested fns, function types, `@discardableResult`, `-> Never`; `if`/`guard`/`while`/
`repeat`/`for-in`/`for case where`; `switch` + value/range/tuple patterns + `where` +
`fallthrough`; labeled break/continue; exhaustiveness/`@unknown default`.
*Exit:* recursion, switch, labeled-loop runtime fixtures pass on Rust backend.

**F3 — Tier 2 (value & nominal types).** `struct`/`enum` (assoc/raw/`indirect`/nested),
memberwise init, methods, `mutating`; properties (stored/computed/observers/`lazy`/
`static`); optionals (`?`,`!`,`?.`,`??`,`if let`/`guard let`, IUO, optional patterns);
subscripts (instance/`static`/overloads); implicit member `.foo`.
*Exit:* all Tier 2 runtime fixtures pass on Rust backend.

**F4 — Tier 3 (classes/ARC/closures).** `class` + inheritance/override/`final`/`super`;
designated/convenience/`required`/failable inits + 2-phase; `weak`/`unowned`(`(unsafe)`);
`===`; casting `is`/`as?`/`as!`/`as`; closures (trailing/shorthand/captures/`@escaping`/
`@autoclosure`/capture-`inout`).
*Exit:* all Tier 3 runtime fixtures pass on Rust backend.

**F5 — Tier 4 (protocols/generics/extensions).** Protocols (inherit/compose/default
impls/assoc types/`Self`/`any`/class-only/conditional conformance/witness for operators);
generics (`<T>`, constraints, `where`, assoc-type constraints, generic subscripts,
contextual `where`); extensions (conditional, on generic types); synthesized
`Equatable`/`Hashable`/`Comparable`. Build our **ConformanceTable + AssocTypeTable +
type-substitution** (msf §9–16 shapes the runtime reads).
*Exit:* generic-stack, protocol-default, conformance runtime fixtures pass on Rust.

**F6 — Tier 5/6/9 (errors, modules, advanced types, attrs, directives).**
`throws`/`try`/`do-catch`/`rethrows`/typed-`throws(E)`/`defer`; opaque `some`/`any`/
metatypes/`type(of:)`/`Self`/implicit-member; access control + custom operators +
`precedencegroup`; attributes the runtime observes (`@main`, `@discardableResult`,
`@propertyWrapper`, `@escaping`, …); **`#if` conditional-compilation evaluation pass**;
`#file`/`#line`/`#column`/`#function`; `#warning`/`#error`; multi-file module driver
(`MSFModule` equivalent).
*Exit:* errors/defer, property-wrapper, Codable, `@main`, `#if`, multifile runtime
fixtures pass on Rust.

**F7 — Cutover.** Flip the default feature to `rust`; delete `crates/msf-sys`,
`build.rs`, `wrapper.h`, `stub.c`; remove `bindgen`/`cc`. Keep C msf only behind a
dev-only `c-oracle` feature for differential CI until confidence is high, then drop the
submodule (§8).

**F8+ — Frontend-gap features the C msf couldn't do (now ours to own).** Parameter
packs / variadic generics `each`, integer generic params, `~Copyable`/`~Escapable`,
key paths, `@dynamicMemberLookup`/`@dynamicCallable`, macros (`#macro`/`@Macro`/
`@resultBuilder`), full concurrency syntax (`async`/`await`/`actor`/`@MainActor`/
`Sendable`/`for await`). These map to runtime Tiers 6+/7/8.

---

## 5. Golden test strategy (the spec — built first, see §9)

Frontend fixtures live in **`vendor/msf/tests/swift-fixtures/`** (extending the existing
`ok/`, `parse/`, `sema/` convention) and use the established directive language:

- `// expected-no-diagnostics` (file scope) — must lex+parse+type **clean**.
- `// expected-error{{substring}}` (line scope) — a diagnostic whose message contains
  `substring` must be reported on/near that line (asymmetric match, msf's rule).

We add **tier-organised subdirectories** so coverage maps 1:1 onto the checklist:

```
vendor/msf/tests/swift-fixtures/
├── ok/ parse/ sema/              # existing legacy fixtures (kept)
├── tier0-lexical/                # literals, comments, operators, ranges, identity
├── tier1-imperative/             # bindings, functions, control flow, switch
├── tier2-value-types/            # struct/enum/optional/subscript/properties
├── tier3-reference-arc/          # class/ARC/inheritance/closures
├── tier4-protocols-generics/     # protocols/generics/extensions
├── tier5-errors-modules/         # throws/defer/#if/@main/property-wrappers
├── tier6-advanced-types/         # some/any/metatype/keypath
├── tier7-concurrency/            # async/await/actor (F8+ frontend gap)
├── tier8-macros/                 # #macro/@Macro/@resultBuilder (F8+)
├── tier9-attributes-operators/   # access control/custom operators/attributes/directives
└── tier10-stdlib/               # stdlib type *shapes* parse+resolve
```

**Three fixture flavours** (a fixture may carry all three expectations):

1. **Parse/lex correctness** — positive (`expected-no-diagnostics`) and negative
   (`parse/` style `expected-error`, e.g. unterminated string, deep nesting).
2. **Sema correctness** — type mismatches, `let` reassignment, undeclared types, etc.
   (`sema/` style `expected-error`).
3. **AST snapshot** — for shape-sensitive features, a sibling `<name>.ast` golden of
   `Node::dump()` (s-expr) asserts the *structure*, not just the absence of errors. The
   harness writes these with `UPDATE_EXPECT=1` and diffs otherwise.

**Two harnesses** (Rust, replacing msf's `test_swift_fixtures.c`):
- `frontend-fixtures` test in `tswift-frontend`: walk the tree, run `analyze`,
  assert diagnostics satisfy the directives and `.ast` snapshots match.
- The existing runtime golden harness (`crates/tswift-cli/tests/golden.rs`)
  continues to validate end-to-end *behaviour* — unchanged.

**Definition of done per checklist row:** ≥1 frontend fixture (parse+sema clean, or the
documented expected diagnostic) **and**, where the runtime implements it, the existing
behaviour fixture — both green on the Rust backend, and (while it exists) **diff-clean
vs the C oracle** (§6).

---

## 6. Differential testing vs the C oracle (our biggest advantage)

We already vendor a working reference frontend. Exploit it: a `differential` test
compiles **both** backends and, for every fixture in `swift-fixtures/` *and* every
`*.swift` in the runtime fixtures, asserts:

1. **Diagnostics agree** — same set of error lines (message wording may differ; match on
   presence + line + a normalised category, not exact text).
2. **AST agrees** — normalised `Node::dump()` from each backend is identical (modulo a
   documented allow-list of intentional divergences, e.g. our promoted `NodeKind`s).

This turns "did my Rust parser get this right?" into a mechanical, exhaustive check
against ~30 years of Swift-grammar edge cases already encoded in msf — for free, until
we delete it. CI matrix: `--features c-oracle` vs `--features rust`, plus the
`differential` job that links both.

> Where the oracle is *wrong* or *absent* (the checklist's "FE gaps"), there is nothing
> to diff against: those fixtures carry a `// oracle-gap:` marker, are authored from
> TSPL/`swiftc` ground truth, and the differential job **skips** the C oracle for them.

### 6.1 Oracle gaps already discovered (bootstrap finding)

Validating the §9 bootstrap fixtures through the current C msf: **34/40** positive
fixtures resolve clean; the remaining **6 are valid Swift the C msf cannot parse** —
concrete evidence of where the Rust frontend must do better. They are marked
`// oracle-gap:` and become Rust-frontend acceptance tests:

| Fixture | Gap in C msf |
|---|---|
| `tier0-lexical/regex_literals.swift` | does not lex `/.../`, `#/.../#` regex literals |
| `tier1-imperative/for_case_optional.swift` | rejects `?` optional pattern in `for case` |
| `tier4-protocols-generics/generics.swift` | stdlib `Sequence` not in baked vocab |
| `tier7-concurrency/concurrency.swift` | `AsyncSequence` + concurrency syntax (F8+) |
| `tier8-macros/macros.swift` | macro syntax (F8+) |
| `tier9-attributes-operators/custom_operators.swift` | does not lex unicode operator chars (`√`, `°`) |

---

## 7. Risks & mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Swift grammar is huge; subtle parse edge cases | High | Differential oracle (§6) catches divergence exhaustively; tier-by-tier milestones |
| Operator precedence / `precedencegroup` user-defined | Med | Port the *algorithm* (Pratt + precedence graph), study msf's `pratt.c`; dedicated fixtures |
| Type inference depth (overloads, generics, literals) | High | Stage F5; lean on oracle diff; scope to checklist's resolved-`TypeRef` needs, not full Hindley-Milner |
| Runtime silently depends on a C-AST quirk | High | Step 0 (§2.1) removes the known one; differential AST diff surfaces the rest before cutover |
| NFC / grapheme / Unicode version skew | Med | Pin `unicode-normalization`/`-segmentation` to Swift's Unicode version; fixture per tricky cluster |
| Perf regression vs SWAR lexer | Low/Med | Correctness first; revisit with our own SIMD later; benchmark gate before declaring done |
| Scope creep into SDK vocabulary | Med | Own a *small* stdlib-shape table (§3.3); expand only by demand-driven fixtures |
| Big-bang cutover breaks runtime | High | Dual-backend feature flag; flip only when differential + all fixtures green |

---

## 8. Decommissioning msf

After F7 cutover and a soak period with the `c-oracle` differential job green:
1. Delete `crates/msf-sys` (`build.rs`, `wrapper.h`, `stub.c`, bindings).
2. Drop `bindgen`/`cc` from the dependency tree.
3. Remove the `vendor/msf` submodule and `.gitmodules` entry **except** the
   `tests/swift-fixtures/` corpus, which we **de-vendor into the repo** (e.g. move to
   `tests/swift-fixtures/` at the workspace root) since those golden tests are now
   *ours*. Preserve git history of the fixtures.
4. Update `AGENTS.md`, `README.md`, and the implementation plan to drop FFI language.
5. Confirm **no crate, path, or `use` named `msf` remains** — the public crate is
   `tswift-frontend`, the engine crates are `swift-*`. The only surviving `msf`
   reference is historical (research docs).

> The fixtures currently sit inside the `vendor/msf` **submodule**; new fixtures we add
> there are awkward to commit. Recommended: in F0, **copy** the `ok/parse/sema` corpus
> out of the submodule into a repo-owned `tests/swift-fixtures/` and author all new tier
> fixtures there, so the spec lives in our repo from day one. (This plan writes them to
> the requested `vendor/msf/tests/swift-fixtures/` path; relocate during F0 if the
> submodule blocks commits.)

---

## 9. Coverage matrix — checklist row → fixture

Every `feature-checklist.md` row maps to a fixture file below. `created` = authored by
this plan's bootstrap; `TODO` = to be authored during the owning milestone. (A single
file intentionally covers several closely-related rows.)

### Tier 0 — Lexical & Literals  → `tier0-lexical/`
| Rows | Fixture | State |
|---|---|---|
| int/float/bool/nil/string/multiline/raw/extended-delim literals | `literals.swift` | created |
| string interpolation | `string_interpolation.swift` | created |
| regex literals `/.../`, `#/.../#` | `regex_literals.swift` | created |
| unicode identifiers + NFC | `unicode_identifiers.swift` | created |
| comments (line/block/nested/doc) | `comments.swift` | created |
| operators arith/cmp/logical/bitwise | `operators.swift` | created |
| wrapping `&+ &- &* &<< &>>` | `wrapping_operators.swift` | created |
| ranges `..<` `...` (+ one-sided), nil-coalescing `??` | `ranges_and_coalescing.swift` | created |
| identity `===` `!==` | `identity_operators.swift` | created |
| unterminated string / block comment (negative) | `parse/` legacy + `bad_lexical.swift` | created |

### Tier 1 — Core Imperative  → `tier1-imperative/`
| Rows | Fixture | State |
|---|---|---|
| let/var, inference, compound-assign, ternary, tuples, wildcard, int conv | `bindings_and_exprs.swift` | created |
| functions, labels, defaults, variadics, inout, nested, fn-types, `-> Never`, `@discardableResult` | `functions.swift` | created |
| if/guard/while/repeat/for-in/for-case-where | `control_flow.swift` | created |
| switch + value/range/tuple patterns + where + fallthrough + labels + `@unknown default` | `switch.swift` | created |
| `let` reassignment error (negative) | `sema/` legacy + `bad_imperative.swift` | created |

### Tier 2 — Value & Nominal Types  → `tier2-value-types/`
| Rows | Fixture | State |
|---|---|---|
| struct, stored props, memberwise init, methods, `mutating`, nested types | `structs.swift` | created |
| enum simple/assoc/raw/`indirect`/methods/`CaseIterable` | `enums.swift` | created |
| properties: computed/observers/`lazy`/`static`/global | `properties.swift` | created |
| optionals: `?`,`!`,`?.`,`??`, if/guard let, IUO, optional pattern | `optionals.swift` | created |
| subscripts: instance/static/overloads | `subscripts.swift` | created |
| mutate `let` member (negative) | `sema/` legacy | exists |

### Tier 3 — Reference Types & Memory  → `tier3-reference-arc/`
| Rows | Fixture | State |
|---|---|---|
| class, inheritance, override, `final`, `super`, dynamic dispatch | `classes.swift` | created |
| inits: designated/convenience/`required`/failable/2-phase | `initializers.swift` | created |
| `weak`/`unowned`/`unowned(unsafe)`, identity, casting `is`/`as?`/`as!` | `memory_and_casting.swift` | created |
| closures: trailing/shorthand/captures/`@escaping`/`@autoclosure`/capture-list | `closures.swift` | created |

### Tier 4 — Protocols, Generics, Extensions  → `tier4-protocols-generics/`
| Rows | Fixture | State |
|---|---|---|
| protocols: decl/inherit/compose/default/assoc-type/`Self`/`any`/class-only/conditional | `protocols.swift` | created |
| generics: `<T>`/constraints/`where`/assoc-constraints/generic-subscript/contextual-where | `generics.swift` | created |
| extensions: methods/inits/conformance/conditional/on-generic | `extensions.swift` | created |
| synthesized Equatable/Hashable/Comparable | `synthesized_conformances.swift` | created |

### Tier 5 — Errors, Resources, Modules  → `tier5-errors-modules/`
| Rows | Fixture | State |
|---|---|---|
| Error/throws/throw/do-catch/try/try?/try!/rethrows/typed-throws/defer | `error_handling.swift` | created |
| property wrappers + projected `$` | `property_wrappers.swift` | created |
| `@main`, access control, `#file`/`#line`, `#warning`/`#error` | `directives_and_main.swift` | created |
| `#if`/`#elseif`/`#else`/`#endif` + `canImport`/`swift()` | `conditional_compilation.swift` | created |

### Tier 6 — Advanced Types  → `tier6-advanced-types/`
| Rows | Fixture | State |
|---|---|---|
| opaque `some`/boxed `any`/metatypes/`type(of:)`/`Self`/implicit-member | `opaque_any_metatype.swift` | created |
| key paths `\Root.path` + as functions | `key_paths.swift` | created |

### Tier 9 — Attributes & Operators  → `tier9-attributes-operators/`
| Rows | Fixture | State |
|---|---|---|
| prefix/infix/postfix operator decls + `precedencegroup` + overloading | `custom_operators.swift` | created |
| attributes (`@available`/`@frozen`/`@inlinable`/`@Sendable`/…) parse | `attributes.swift` | created |

### Tier 10 — Stdlib surface (shapes parse + resolve)  → `tier10-stdlib/`
| Rows | Fixture | State |
|---|---|---|
| Int/UInt widths, Float/Double, Bool, String/Character/Substring, Range | `core_value_types.swift` | created |
| Array/Dictionary/Set/ContiguousArray/ArraySlice | `collections.swift` | created |
| protocol surface: Sequence/Collection/Codable/Identifiable/etc. signatures | `stdlib_protocols.swift` | created |

### Tier 7 / Tier 8 — Concurrency & Macros (F8+ frontend gaps)  → `tier7-concurrency/`, `tier8-macros/`
| Rows | Fixture | State |
|---|---|---|
| async/await/async-let/Task/TaskGroup/actor/@MainActor/nonisolated/Sendable/for-await | `concurrency.swift` | created |
| freestanding `#macro` / attached `@Macro` / macro decl / `@resultBuilder` | `macros.swift` | created |

---

## 10. Immediate next actions

1. **Step 0** — (a) promote runtime's `NodeKind::Other(10/11/27/28)` matches to named
   variants (`ImportDecl`/`TypeAliasDecl`/`PrecedenceGroupDecl`/`OperatorDecl`); and
   (b) rename `crates/msf` → `tswift-frontend`, updating the `use msf::…` sites in
   `tswift-core`/`-std`. Keep the C backend; verify green. (Decouples us from msf's
   integers *and* its name before any engine work.)
2. **Bootstrap the golden fixtures** (§9) under `vendor/msf/tests/swift-fixtures/` — done
   alongside this plan.
3. **Stand up the differential harness** (§6) so every new fixture is checked against the
   C oracle from the first commit.
4. **Scaffold** `tswift-lexer` and start F1 (Tier 0 lexical), diffing tokens/AST vs oracle.
5. Convert `feature-checklist.md` "FE" column statuses into F0–F8 issues.
