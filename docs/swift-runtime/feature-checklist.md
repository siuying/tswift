# Swift Runtime — Complete Feature Checklist

**Goal:** support **every** Swift language feature, end to end (parse → typecheck →
run), on top of the typed AST.

> **Frontend cutover (#56, done):** the default frontend is now the **pure-Rust**
> pipeline (`tswift-lexer`/`-ast`/`-parser`/`-sema` → `tswift-frontend::compat`).
> The vendored `msf` C frontend, `msf-sys`, `bindgen`, `cc`, and the C submodule have
> been **removed** — the default build/test needs no C toolchain. All 53 runtime
> fixtures pass on the Rust backend with no Rust-vs-msf AST special cases in
> `tswift-core`/`-std`. The golden fixture harness now validates **every**
> positive `tests/swift-fixtures` file against the pure-Rust frontend — the
> `// rust-gap:` escape hatch has been removed and those fixtures now parse and
> type-check cleanly. The **FE** column below records the historical msf status;
> frontend gaps are now ours to close.

**Reference:** *The Swift Programming Language* (TSPL), **Swift 6.3** —
`github.com/swiftlang/swift-book` (Language Guide: 28 chapters; Reference Manual:
Lexical Structure, Types, Expressions, Statements, Declarations, Attributes, Patterns,
Generics; 34 declaration/type attributes).

**How to read the columns**
- **FE** = msf **frontend** status (does the parser/sema already handle it?):
  - ✅ parsed + typed by msf today
  - ⚠️ partially parsed / needs verification
  - ❌ not in msf — **frontend work required first**
- **RT** = **runtime** implementation complexity: ★ trivial · ★★ moderate · ★★★ hard · ★★★★ very hard
- **Phase** = target phase from the runtime design doc (R0–R6); `+` = post-R6

> Cross-ref: `docs/research/2026-06-24-implementing-a-swift-runtime-on-msf-ast.md`
> (architecture) and `docs/research/2026-06-24-msf-swift-frontend.md` (frontend).

Legend for status of each checkbox: `[ ]` todo · `[~]` in progress · `[x]` done.

---

## Tier 0 — Lexical & Literals (foundation, must be 100%)

*Everything downstream depends on these. msf's lexer already covers them.*

| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | Integer literals (dec/hex/oct/bin, `_` separators) | ✅ | ★ | R0 |
| [x] | Floating-point literals (dec/hex, exponents) | ✅ | ★ | R0 |
| [x] | Boolean literals `true`/`false` | ✅ | ★ | R0 |
| [x] | `nil` literal | ✅ | ★ | R0 |
| [x] | String literals (escapes, `\u{}`) | ✅ | ★ | R0 |
| [x] | Multiline string literals `"""` | ✅ | ★ | R0 |
| [x] | Raw string literals `#"..."#` | ✅ | ★ | R0 |
| [x] | String interpolation `\(expr)` (re-parsed by the Rust frontend) | ✅ | ★★ | R1 |
| [x] | Extended string delimiters `#"\n"#` | ✅ | ★ | R1 |
| [x] | Regex literals `/.../ ` and `#/.../#` | ✅ | ★★★ | R5+ |
| [x] | Unicode identifiers | ✅ | ★ | R0 |
| [x] | Comments (line, block, nested, doc) | ✅ | ★ | R0 |
| [x] | Operators: arithmetic/comparison/logical/bitwise/range | ✅ | ★ | R0 |
| [x] | Wrapping operators `&+ &- &*` (+ `&<<` `&>>`) | ✅ | ★★ | R1 |
| [x] | Overflow-trapping integer semantics | ✅ | ★★ | R1 |
| [x] | Nil-coalescing `??` | ✅ | ★ | R2 |
| [x] | Range operators `..<` `...` (+ one-sided) | ✅ | ★★ | R1 |
| [x] | Identity operators `===` `!==` | ✅ | ★ | R3 |

---

## Tier 1 — Core Imperative (MVP: run real programs)

*The minimal language that runs straight-line code, functions, and control flow.*

### 1a. Bindings & basic expressions
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | `let` / `var` declarations + type annotations | ✅ | ★ | R0 |
| [x] | Type inference for initializers | ✅ | ★ | R0 |
| [x] | Arithmetic / comparison / logical / bitwise eval | ✅ | ★ | R0 |
| [x] | Compound assignment `+= -= *= …` | ✅ | ★ | R0 |
| [x] | Ternary `a ? b : c` | ✅ | ★ | R1 |
| [x] | Tuples + tuple decomposition `let (a,b) = …` | ✅ | ★★ | R1 |
| [x] | Parenthesized / wildcard `_` expressions | ✅ | ★ | R1 |
| [x] | Integer width conversions & `Int(x)` casts | ✅ | ★★ | R1 |

### 1b. Functions
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | Function declarations, params, return | ✅ | ★★ | R0 |
| [x] | Argument labels & parameter names | ✅ | ★★ | R1 |
| [x] | Default parameter values | ✅ | ★★ | R1 |
| [x] | Variadic parameters `T...` | ✅ | ★★ | R1 |
| [x] | `inout` parameters (true lvalue aliasing) | ✅ | ★★★ | R2 |
| [x] | Nested functions + capture | ✅ | ★★ | R3 |
| [x] | Function types as values / params / returns | ✅ | ★★ | R3 |
| [x] | Multiple return values via tuples (positional and named element access) | ✅ | ★ | R1 |
| [x] | `@discardableResult` | ✅ | ★ | R1 |
| [x] | Functions that never return (`-> Never`) | ✅ | ★★ | R2 |

### 1c. Control flow
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | `if` / `else` / `else if` | ✅ | ★ | R0 |
| [x] | `if` as expression (Swift 5.9) | ✅ | ★★ | R2 |
| [x] | `guard` / `guard let` | ✅ | ★★ | R2 |
| [x] | `while` / `repeat-while` | ✅ | ★ | R1 |
| [x] | `for-in` over ranges/arrays/sequences | ✅ | ★★ | R1 |
| [x] | `for case` / `for ... where` | ✅ | ★★ | R2 |
| [x] | `switch` + cases + `default` | ✅ | ★★★ | R1 |
| [x] | `switch` value/range/tuple patterns | ✅ | ★★★ | R2 |
| [x] | `where` clauses in cases (`cas.where_expr`) | ✅ | ★★ | R2 |
| [x] | `fallthrough` | ✅ | ★★ | R1 |
| [x] | `break` / `continue` + labeled statements | ✅ | ★★ | R1 |
| [x] | `switch` exhaustiveness / `@unknown default` | ✅ | ★★ | R4 |

---

## Tier 2 — Value & Nominal Types

### 2a. Structures & Enumerations (value types)
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | `struct` declaration + stored properties | ✅ | ★★ | R2 |
| [x] | **Value semantics** (copy on assign/pass) | ✅ | ★★★ | R2 |
| [x] | Memberwise initializers (synthesized) | ✅ | ★★ | R2 |
| [x] | Methods on structs | ✅ | ★★ | R2 |
| [x] | `mutating` methods (inout self) | ✅ | ★★★ | R2 |
| [x] | `enum` with simple cases | ✅ | ★★ | R2 |
| [x] | Enum **associated values** | ✅ | ★★★ | R2 |
| [x] | Enum **raw values** + `RawRepresentable` | ✅ | ★★ | R2 |
| [x] | `indirect` enums (recursive) | ⚠️ | ★★★ | R3 |
| [x] | Enum methods / computed props | ✅ | ★★ | R2 |
| [x] | `CaseIterable` synthesis | ✅ | ★★ | R4 |
| [x] | Nested types | ✅ | ★★ | R2 |

### 2b. Properties
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | Stored properties (let/var) | ✅ | ★ | R2 |
| [x] | Computed properties (get/set) | ✅ | ★★ | R2 |
| [x] | Read-only computed properties | ✅ | ★ | R2 |
| [x] | Property observers `willSet`/`didSet` | ✅ | ★★ | R3 |
| [x] | `lazy` stored properties | ✅ | ★★ | R3 |
| [x] | Type properties `static`/`class` | ✅ | ★★ | R2 |
| [x] | Property wrappers `@propertyWrapper` | ✅ | ★★★ | R5 |
| [x] | Projected values `$wrapper` | ⚠️ | ★★★ | R5 |
| [ ] | Global & local variables (lazy globals) | ✅ | ★★ | R2 |

### 2c. Optionals
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | Optional type `T?` | ✅ | ★★ | R2 |
| [x] | `if let` / `guard let` binding | ✅ | ★★ | R2 |
| [x] | Shorthand `if let x` (Swift 5.7) | ✅ | ★ | R2 |
| [x] | Forced unwrap `!` (trap on nil) | ✅ | ★ | R2 |
| [x] | Optional chaining `?.` | ✅ | ★★★ | R2 |
| [x] | Nil-coalescing `??` | ✅ | ★ | R2 |
| [x] | Implicitly unwrapped optionals `T!` | ✅ | ★★ | R2 |
| [x] | `Optional` pattern `case let x?` | ✅ | ★★ | R2 |

### 2d. Subscripts
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | Instance subscripts | ✅ | ★★ | R3 |
| [x] | Type subscripts (`static subscript`) | ✅ | ★★ | R3 |
| [x] | Subscript overloads / multi-param | ✅ | ★★ | R3 |

---

## Tier 3 — Reference Types & Memory (ARC)

| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | `class` declaration + reference semantics | ✅ | ★★★ | R3 |
| [x] | **ARC retain/release** (refcount, no cycle GC) | ✅ | ★★★ | R3 |
| [x] | Deterministic `deinit` at refcount 0 | ✅ | ★★ | R3 |
| [x] | Inheritance + method/property override | ✅ | ★★★ | R3 |
| [x] | `final` / `override` semantics | ✅ | ★★ | R3 |
| [x] | Dynamic dispatch (vtables) | ✅ | ★★★ | R3 |
| [x] | `super.` calls | ✅ | ★★ | R3 |
| [~] | Designated / convenience initializers | ✅ | ★★★ | R3 |
| [~] | Initializer delegation + 2-phase init | ✅ | ★★★★ | R3 |
| [x] | `required` initializers | ✅ | ★★ | R3 |
| [x] | Failable initializers `init?` / `init!` | ✅ | ★★ | R3 |
| [x] | `weak` references (zeroing side table) | ✅ | ★★★ | R3 |
| [x] | `unowned` references | ✅ | ★★ | R3 |
| [x] | `unowned(unsafe)` | ⚠️ | ★★ | R3 |
| [x] | Identity `===` `!==` | ✅ | ★ | R3 |
| [x] | Type casting `is` / `as?` / `as!` / `as` | ✅ | ★★★ | R3 |
| [x] | Downcasting in class hierarchies | ✅ | ★★★ | R3 |

### 3a. Closures
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | Closure expressions | ✅ | ★★★ | R3 |
| [x] | Trailing closures (+ multiple) | ✅ | ★★ | R3 |
| [x] | Shorthand args `$0 $1` | ✅ | ★★ | R3 |
| [x] | Capture by reference (open/closed upvalues) | ✅ | ★★★ | R3 |
| [~] | Capture lists `[weak self]` `[unowned]` | ✅ | ★★★ | R3 |
| [~] | `@escaping` closures | ✅ | ★★★ | R3 |
| [x] | `@autoclosure` | ✅ | ★★ | R3 |
| [x] | Closures capturing `inout` | ✅ | ★★★ | R3 |

---

## Tier 4 — Abstraction: Protocols, Generics, Extensions

### 4a. Protocols
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | Protocol declaration (methods/props/init) | ✅ | ★★★ | R4 |
| [x] | Conformance + **witness tables** (msf ConformanceTable) | ✅ | ★★★ | R4 |
| [~] | Protocol inheritance | ✅ | ★★ | R4 |
| [x] | Protocol composition `P & Q` | ✅ | ★★ | R4 |
| [x] | Default implementations (in extensions) | ✅ | ★★★ | R4 |
| [x] | Associated types (msf AssocTypeTable) | ✅ | ★★★ | R4 |
| [x] | Protocol as type / existential `any P` | ✅ | ★★★ | R4 |
| [x] | `Self` requirements | ✅ | ★★★ | R4 |
| [x] | Protocol witness for operators | ✅ | ★★ | R4 |
| [ ] | Optional protocol requirements (`@objc optional`) | ⚠️ | ★★★ | R4+ |
| [x] | Class-only protocols (`AnyObject`) | ✅ | ★★ | R4 |
| [x] | Conditional conformance | ✅ | ★★★ | R4 |
| [~] | Synthesized `Equatable`/`Hashable`/`Comparable` | ✅ | ★★★ | R4 |
| [~] | Synthesized `Codable` (Encodable/Decodable) | ✅ | ★★★★ | R5 |

### 4b. Generics
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | Generic functions `<T>` | ✅ | ★★★ | R4 |
| [x] | Generic types (struct/class/enum) | ✅ | ★★★ | R4 |
| [x] | Type constraints `<T: Protocol>` | ✅ | ★★★ | R4 |
| [~] | `where` clauses (msf `type_substitute`) | ✅ | ★★★ | R4 |
| [x] | Associated-type constraints | ✅ | ★★★ | R4 |
| [x] | Generic subscripts | ✅ | ★★ | R4 |
| [ ] | Monomorphization vs witness dispatch | ✅ | ★★★★ | R4 |
| [x] | Contextual `where` on extensions | ✅ | ★★★ | R4 |
| [ ] | Parameter packs / variadic generics `each` | ⚠️ | ★★★★ | R6+ |
| [ ] | Integer generic parameters (`let N: Int`) | ⚠️ | ★★★ | R6+ |
| [ ] | `~Copyable` / `~Escapable` (suppressed constraints) | ✅ | ★★★★ | R6+ |

### 4c. Extensions
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | Extend struct/class/enum/protocol | ✅ | ★★ | R4 |
| [x] | Add methods/computed props/inits/subscripts | ✅ | ★★ | R4 |
| [x] | Add protocol conformance via extension | ✅ | ★★★ | R4 |
| [x] | Conditional extensions (`where`) | ✅ | ★★★ | R4 |
| [x] | Extensions on generic types | ✅ | ★★★ | R4 |

---

## Tier 5 — Error Handling & Resource Management

| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | `Error` protocol + custom errors | ✅ | ★★ | R5 |
| [x] | `throws` functions | ✅ | ★★★ | R5 |
| [x] | `throw` statement | ✅ | ★★ | R5 |
| [x] | `do` / `catch` (+ pattern catches) | ✅ | ★★★ | R5 |
| [x] | `try` / `try?` / `try!` | ✅ | ★★ | R5 |
| [~] | `rethrows` | ✅ | ★★★ | R5 |
| [~] | Typed throws `throws(E)` (Swift 6) | ⚠️ | ★★★ | R5 |
| [x] | `defer` statements (LIFO on scope exit) | ✅ | ★★ | R5 |
| [x] | Error propagation through call stack | ✅ | ★★★ | R5 |
| [x] | `Result<Success, Failure>` (stdlib) | ✅ | ★★ | R5 |

---

## Tier 6 — Advanced Types & Expressions

| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | Opaque types `some P` (return) | ✅ | ★★★ | R4 |
| [x] | Boxed/existential `any P` | ✅ | ★★★ | R4 |
| [x] | Metatypes `T.self` / `.Type` / `.Protocol` | ✅ | ★★★ | R4 |
| [x] | `type(of:)` dynamic type | ✅ | ★★ | R4 |
| [x] | Key paths `\Root.path` | ✅ | ★★★ | R6+ |
| [x] | Key-path expressions as functions | ⚠️ | ★★★ | R6+ |
| [x] | `@dynamicMemberLookup` | ✅ | ★★★ | R6+ |
| [x] | `@dynamicCallable` | ⚠️ | ★★★ | R6+ |
| [ ] | `#selector` / `#keyPath` | ⚠️ | ★★ | R6+ |
| [x] | Self type | ✅ | ★★ | R4 |
| [x] | Implicit member expr `.foo` | ✅ | ★★ | R2 |
| [ ] | `consume` / `borrow` operators (ownership) | ✅ | ★★★★ | R6+ |
| [ ] | `discard self` | ✅ | ★★★ | R6+ |

---

## Tier 7 — Concurrency

*Runs on a single-threaded cooperative executor (ADR-0005) over the suspension
primitive (ADR-0004, `corosensei`). Suspension is decoupled from the #11 bytecode
VM, so the tree-walker handles concurrency directly. The executor uses
run-to-completion-at-await scheduling: results and structure are faithful;
preemptive interleaving order may differ (documented in ADR-0005).*

| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | `async` functions | ✅ | ★★★★ | R6+ |
| [x] | `await` expressions | ✅ | ★★★★ | R6+ |
| [x] | `async let` | ⚠️ | ★★★★ | R6+ |
| [x] | `Task` / `Task.detached` | ✅(stdlib) | ★★★★ | R6+ |
| [x] | Task groups (`withTaskGroup`) | ✅(stdlib) | ★★★★ | R6+ |
| [~] | Task cancellation | n/a | ★★★ | R6+ |
| [x] | `actor` declarations + isolation | ✅ | ★★★★ | R6+ |
| [~] | Actor reentrancy / serial executor | n/a | ★★★★ | R6+ |
| [x] | `@MainActor` / global actors | ✅ | ★★★★ | R6+ |
| [~] | `nonisolated` / `isolated` params | ✅ | ★★★ | R6+ |
| [~] | `Sendable` checking | ✅ | ★★★ | R6+ |
| [x] | `AsyncSequence` / `for await` | ⚠️ | ★★★★ | R6+ |
| [ ] | Continuations (`withCheckedContinuation`) | n/a | ★★★★ | R6+ |
| [ ] | Strict concurrency (Swift 6 mode) | ✅ | ★★★ | R6+ |

**Implemented this milestone (#12):** `async`/`await`, `async let`,
`Task`/`Task.detached` (+ `.value`/`.cancel()`/`.isCancelled`), `withTaskGroup` +
`addTask` + `for await` aggregation + `cancelAll()`, `actor` declarations
(serialized for free on one thread), `@MainActor`/global-actor annotations
(accepted; run on the cooperative main), custom `AsyncSequence`/`for await`.
`Sendable` and `@Sendable` are **accepted and parsed but not statically checked**
(every value is effectively sendable on one thread). **Gaps:** preemptive
ordering / `Task.yield` interleaving (the `corosensei` primitive in
`suspend.rs` is the migration path), continuations, and strict-concurrency
diagnostics. See ADR-0005 for the fidelity boundary.

---

## Tier 8 — Macros & Metaprogramming (Swift 5.9+)

*Heavy: real macros run a separate compiler plugin (SwiftSyntax). A faithful runtime
needs a macro-expansion engine over the AST before evaluation.*

| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | Freestanding macros `#macro` | ⚠️ | ★★★★ | R6+ |
| [ ] | Attached macros `@Macro` | ⚠️ | ★★★★ | R6+ |
| [ ] | Macro declarations | ✅(AST_MACRO_DECL) | ★★★★ | R6+ |
| [x] | Built-in `#file`/`#line`/`#function`/`#column` | ⚠️ | ★★ | R5 |
| [~] | `#warning` / `#error` | ⚠️ | ★ | R1 |
| [ ] | `@freestanding` / `@attached` roles | ⚠️ | ★★★★ | R6+ |
| [ ] | `@resultBuilder` (DSL transform) | ⚠️ | ★★★★ | R6+ |
| [ ] | Result-builder method synthesis | ❌ | ★★★★ | R6+ |

---

## Tier 9 — Attributes, Access Control, Operators, Directives

### 9a. Access control
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | `open` `public` `internal` `fileprivate` `private` | ✅ | ★★ | R4 |
| [x] | `package` access level | ✅ | ★★ | R4 |
| [x] | Access on setters `private(set)` | ✅ | ★★ | R4 |
| [~] | Module boundaries / `import` | ✅ | ★★★ | R5 |

### 9b. Custom operators
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | `prefix`/`infix`/`postfix` operator decls | ✅ | ★★★ | R4 |
| [x] | `precedencegroup` (+ `higherThan`/`assoc`) | ✅ | ★★★ | R4 |
| [x] | Operator method implementations | ✅ | ★★ | R4 |
| [x] | Operator overloading | ✅ | ★★ | R4 |

### 9c. Attributes (34 declaration/type attributes)
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | `@available` (+ availability conditions `#available`) | ✅ | ★★ | R5 |
| [x] | `@objc` / `@nonobjc` / `@objcMembers` | ⚠️ | ★★ | R6+ |
| [x] | `@main` entry point | ✅ | ★★ | R5 |
| [x] | `@frozen` / `@inlinable` / `@usableFromInline` | ⚠️ | ★ | R6+ |
| [x] | `@inline` / `@_optimize` (perf hints) | ⚠️ | ★ | R6+ |
| [ ] | `@discardableResult` | ✅ | ★ | R1 |
| [ ] | `@propertyWrapper` | ✅ | ★★★ | R5 |
| [ ] | `@resultBuilder` | ⚠️ | ★★★★ | R6+ |
| [x] | `@globalActor` | ⚠️ | ★★★★ | R6+ |
| [x] | `@Sendable` | ✅ | ★★ | R6+ |
| [ ] | `@autoclosure` / `@escaping` / `@convention` | ✅ | ★★ | R3 |
| [x] | `@dynamicMemberLookup` / `@dynamicCallable` | ✅/⚠️ | ★★★ | R6+ |
| [x] | `@preconcurrency` / `@unchecked` | ✅ | ★★ | R6+ |
| [x] | `@NSCopying` / `@NSManaged` / IB attrs | ⚠️ | ★★ | R6+ |
| [ ] | `@backDeployed` / `@_specialize` | ⚠️ | ★ | R6+ |
| [x] | `@warn_unqualified_access` / misc diagnostics | ⚠️ | ★ | R6+ |

### 9d. Compiler control / directives
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | Conditional compilation `#if`/`#elseif`/`#else`/`#endif` | ⚠️ | ★★ | R5 |
| [x] | `#if` platform/arch/compiler/`canImport`/`swift()` | ⚠️ | ★★ | R5 |
| [ ] | `#sourceLocation` line control | ⚠️ | ★ | R6+ |
| [x] | `#available` / `#unavailable` conditions | ✅ | ★★ | R5 |

---

## Tier 10 — Standard Library Surface (behaviour, not syntax)

*msf provides the type **shapes** (baked vocab) but **no behaviour** — this is the
biggest sustained effort. Scope deliberately.*

### 10a. Core values
| ✓ | Feature | RT | Phase |
|---|---|----|-------|
| [x] | `Int`/`UInt` family (all widths) + overflow ops | ★★ | R0 |
| [x] | `Float`/`Double` + math | ★★ | R0 |
| [x] | `Bool` | ★ | R0 |
| [~] | `String` (UTF-8, NFC, views) + `Character` | ★★★★ | R1 |
| [~] | `Substring` | ★★★ | R2 |
| [x] | `Optional<Wrapped>` | ★★ | R2 |
| [x] | `Range`/`ClosedRange`/`Stride` | ★★ | R1 |
| [ ] | Tuples | ★★ | R1 |

### 10b. Collections (value semantics + CoW)
| ✓ | Feature | RT | Phase |
|---|---|----|-------|
| [x] | `Array<Element>` + CoW | ★★★★ | R1 |
| [x] | `Dictionary<Key,Value>` + CoW | ★★★★ | R2 |
| [x] | `Set<Element>` + CoW | ★★★ | R2 |
| [x] | `ContiguousArray` / `ArraySlice` | ★★★ | R4 |
| [ ] | `isKnownUniquelyReferenced` (CoW correctness) | ★★★ | R3 |

### 10c. Protocols that drive the language
| ✓ | Feature | RT | Phase |
|---|---|----|-------|
| [x] | `Equatable` / `Hashable` / `Comparable` | ★★★ | R4 |
| [x] | `Sequence` / `IteratorProtocol` | ★★★ | R4 |
| [ ] | `Collection` / `BidirectionalCollection` / `RandomAccess` | ★★★★ | R4 |
| [ ] | `RangeReplaceableCollection` | ★★★ | R4 |
| [x] | `ExpressibleBy*Literal` (literal conversion) | ★★★ | R2 |
| [x] | `CustomStringConvertible` / `Debug…` | ★★ | R2 |
| [x] | `RawRepresentable` / `CaseIterable` | ★★ | R2 |
| [x] | `Codable` / `Encodable` / `Decodable` | ★★★★ | R5 |
| [x] | `Identifiable` | ★ | R4 |
| [ ] | `Sendable` | ★★ | R6+ |
| [ ] | `AsyncSequence` / `AsyncIteratorProtocol` | ★★★★ | R6+ |

### 10d. Functions & utilities
| ✓ | Feature | RT | Phase |
|---|---|----|-------|
| [x] | `print` / `debugPrint` / `dump` | ★ | R0 |
| [x] | `map`/`filter`/`reduce`/`flatMap`/`compactMap`/`sorted`/… | ★★★ | R4 |
| [x] | `assert`/`precondition`/`fatalError`/`assertionFailure` | ★★ | R1 |
| [x] | `min`/`max`/`abs`/`stride`/`zip`/`swap` | ★★ | R2 |
| [x] | `Result` | ★★ | R5 |
| [x] | `MemoryLayout` | ★★ | R6+ |
| [ ] | `Unsafe*Pointer` family | ★★★★ | R6+ |

---

## Tier 11 — Framework Surface (Foundation and beyond)

*Closed-source Apple frameworks are measured through generated `.swiftinterface`
inventories plus per-framework runtime registries. See
`docs/plan/framework-support.md` and `tools/framework-inventory/`.*

| ✓ | Feature | RT | Phase |
|---|---|----|-------|
| [x] | Framework inventory/coverage loop (`--framework`, scope manifests, registry dumps) | ★★ | R5+ |
| [x] | Foundation proof slice: `Data`/`UUID` constructors and core properties | ★★ | R5+ |
| [ ] | Foundation F1 remainder: `IndexPath` / `IndexSet` | ★★ | R5+ |
| [ ] | Foundation F2: `URL` / `URLComponents` / `URLQueryItem` | ★★★ | R5+ |
| [ ] | SwiftUI measurement descriptor and runtime ADR | ★★★★ | R6+ |

---

## Summary: implementation order at a glance

```
R0  Tier 0 + 1a (lexical, bindings, arithmetic, print)          → runs straight-line code
R1  Tier 1b/1c + ranges + basic stdlib (Array, print, assert)   → functions + control flow
R2  Tier 2 (structs, enums, optionals, subscripts, properties)  → value types
R3  Tier 3 (classes, ARC, inheritance, closures)                → reference types + memory
R4  Tier 4 + 6(opaque/any/metatype) + 9a/9b/9c-core + 10c       → protocols, generics, dispatch
R5  Tier 5 + property wrappers + Codable + @main + #if          → errors, resources, modules
R6  Bytecode VM (perf) — prerequisite for ↓
R6+ Tier 7 (concurrency), Tier 8 (macros), key paths, ownership, packs, unsafe
```

### Frontend (msf) gaps to close first
These show ⚠️/❌ above and likely need **frontend work** before the runtime can run them:
- [x] Verify raw-string / extended-delimiter lexing edge cases (multiline `"""`, `#"…"#`, and `\(…)` with inner quotes are lexed as single tokens)
- [ ] `indirect` enum / recursive layout confirmation
- [ ] Typed throws `throws(E)` parsing
- [ ] Parameter packs / variadic generics (`each`), integer generic params
- [ ] Macro expansion engine (freestanding + attached + result builders)
- [ ] `#if` conditional-compilation evaluation (likely pre-lex or pre-sema pass)
- [ ] `@objc`/IB/ObjC-interop attributes (may be out of scope)
- [ ] Projected values `$wrapper`, `@dynamicCallable` confirmation

### Hardest runtime items (★★★★) — schedule extra time
Value semantics + CoW · 2-phase class init · monomorphization · `String`/`Character`
· `Dictionary`/`Array` CoW · `Codable` synthesis · all of concurrency · macros ·
result builders · parameter packs · `~Copyable` ownership · unsafe pointers.
