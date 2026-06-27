# Feature Checklist Implementation Log

**Goal**: implement all features from docs/swift-runtime/feature-checklist.md
**Signal**: `cargo test --workspace` → all green + per-feature fixtures

## Blocked (needs human input)
See docs/swift-runtime/blocked-features.md

## Iterations

| # | commit | feature | status | notes |
| - | ------ | ------- | ------ | ----- |
| 1 | done | Failable initializers `init?`/`init!` | keep | return nil → Optional.none in struct & class init |
| 2 | done | Nested types + implicit-self mutating writes | keep | register nested decls by simple name; resolve_place implicit self |
| 3 | done | Tuple returns / @discardableResult / Never | keep | already worked; fixtures added |
| 4 | done | Type subscripts (static subscript) | keep | struct+class static_subscript |
| 5 | done | @unknown default | keep | parser accepts attribute before default |
| 6 | done | Metatypes T.self + type(of:) | keep | new SwiftValue::Metatype |
| 7 | ebe0ac1 | `@autoclosure` parameters | keep | parser records attr; thunk-wraps deferred args (free fns + methods) |
| 8 | 86f8a4c | `for case` / `case let x?` refutable patterns | keep | for-loop matches patterns; `x?`→`.some` so switch/for/if filter nil |
| 9 | c3ce916 | multi-binding & while-let conditions | keep | single-binding cond parser; eval_while uses eval_cond_list |
| 10 | 2da5046 | static stored properties + type methods | keep | struct+class statics read/write; static_ctx for unqualified access; static mutating collections. Gap: static computed props |
| 11 | b16cc0b | required init + implicitly-unwrapped optionals | keep | already worked; added runtime fixtures + flipped checklist |
| 12 | 018276e | as/is patterns + custom Error types | keep | `catch let e as T`, `case is T`, `case let x as T`; match_pattern CastExpr |
| 13 | c6fea91 | subscript overloads + nested subscript assign | keep | Vec<SubscriptDef> by arity; get/set; `m[i][j]=v`. Reviewed by subagent (claude-sonnet-4-5), 2 Important fixes applied |
| 14 | pending | verify final/override, deinit, unowned | keep | already worked; added unowned fixture; final/override/deinit covered by existing fixtures |
| 15 | 2be09b5 | Generic subscripts `subscript<T>` | keep | parser accepts `<...>` + `where` on subscript; runtime type-erases. Reviewed by gpt-5.5 (clean). Pre-existing limit: subscript overloads selected by arity only (labels untracked) |
| 16 | 42b5573 | `MemoryLayout<T>` size/stride/alignment | keep | parser records `<T>` as TypeRef child of MemoryLayout; runtime computes 64-bit layout for scalars + structs (C alignment/tail padding); cycle guard. Reviewed by gpt-5.5: pushed back on false NodeKind claim (compat maps TypeRef→TypeIdent); fixed recursion guard + added tail-padding/nested/cyclic tests |
