# Swift Runtime — Complete Feature Checklist

**Goal:** support **every** Swift language feature, end to end (parse → typecheck →
run), on top of msf's typed AST.

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
| [ ] | `nil` literal | ✅ | ★ | R0 |
| [x] | String literals (escapes, `\u{}`) | ✅ | ★ | R0 |
| [x] | Multiline string literals `"""` | ✅ | ★ | R0 |
| [x] | Raw string literals `#"..."#` | ⚠️ | ★ | R0 |
| [ ] | String interpolation `\(expr)` (re-parse via `msf_parse_expression`) | ✅ | ★★ | R1 |
| [ ] | Extended string delimiters `#"\n"#` | ⚠️ | ★ | R1 |
| [ ] | Regex literals `/.../ ` and `#/.../#` | ✅ | ★★★ | R5+ |
| [ ] | Unicode identifiers + NFC normalization (msf vendors NFC) | ✅ | ★ | R0 |
| [ ] | Comments (line, block, nested, doc) | ✅ | ★ | R0 |
| [~] | Operators: arithmetic/comparison/logical/bitwise/range | ✅ | ★ | R0 |
| [x] | Wrapping operators `&+ &- &*` (+ `&<<` `&>>`) | ✅ | ★★ | R1 |
| [x] | Overflow-trapping integer semantics | ✅ | ★★ | R1 |
| [ ] | Nil-coalescing `??` | ✅ | ★ | R2 |
| [ ] | Range operators `..<` `...` (+ one-sided) | ✅ | ★★ | R1 |
| [ ] | Identity operators `===` `!==` | ✅ | ★ | R3 |

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
| [ ] | Tuples + tuple decomposition `let (a,b) = …` | ✅ | ★★ | R1 |
| [ ] | Parenthesized / wildcard `_` expressions | ✅ | ★ | R1 |
| [x] | Integer width conversions & `Int(x)` casts | ✅ | ★★ | R1 |

### 1b. Functions
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [x] | Function declarations, params, return | ✅ | ★★ | R0 |
| [x] | Argument labels & parameter names | ✅ | ★★ | R1 |
| [x] | Default parameter values | ✅ | ★★ | R1 |
| [x] | Variadic parameters `T...` | ✅ | ★★ | R1 |
| [ ] | `inout` parameters (true lvalue aliasing) | ✅ | ★★★ | R2 |
| [x] | Nested functions + capture | ✅ | ★★ | R3 |
| [x] | Function types as values / params / returns | ✅ | ★★ | R3 |
| [ ] | Multiple return values via tuples | ✅ | ★ | R1 |
| [ ] | `@discardableResult` | ✅ | ★ | R1 |
| [ ] | Functions that never return (`-> Never`) | ✅ | ★★ | R2 |

### 1c. Control flow
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | `if` / `else` / `else if` | ✅ | ★ | R0 |
| [ ] | `if` as expression (Swift 5.9) | ✅ | ★★ | R2 |
| [ ] | `guard` / `guard let` | ✅ | ★★ | R2 |
| [ ] | `while` / `repeat-while` | ✅ | ★ | R1 |
| [ ] | `for-in` over ranges/arrays/sequences | ✅ | ★★ | R1 |
| [ ] | `for case` / `for ... where` | ✅ | ★★ | R2 |
| [ ] | `switch` + cases + `default` | ✅ | ★★★ | R1 |
| [ ] | `switch` value/range/tuple patterns | ✅ | ★★★ | R2 |
| [ ] | `where` clauses in cases (`cas.where_expr`) | ✅ | ★★ | R2 |
| [ ] | `fallthrough` | ✅ | ★★ | R1 |
| [ ] | `break` / `continue` + labeled statements | ✅ | ★★ | R1 |
| [ ] | `switch` exhaustiveness / `@unknown default` | ✅ | ★★ | R4 |

---

## Tier 2 — Value & Nominal Types

### 2a. Structures & Enumerations (value types)
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | `struct` declaration + stored properties | ✅ | ★★ | R2 |
| [ ] | **Value semantics** (copy on assign/pass) | ✅ | ★★★ | R2 |
| [ ] | Memberwise initializers (synthesized) | ✅ | ★★ | R2 |
| [ ] | Methods on structs | ✅ | ★★ | R2 |
| [ ] | `mutating` methods (inout self) | ✅ | ★★★ | R2 |
| [ ] | `enum` with simple cases | ✅ | ★★ | R2 |
| [ ] | Enum **associated values** | ✅ | ★★★ | R2 |
| [ ] | Enum **raw values** + `RawRepresentable` | ✅ | ★★ | R2 |
| [ ] | `indirect` enums (recursive) | ⚠️ | ★★★ | R3 |
| [ ] | Enum methods / computed props | ✅ | ★★ | R2 |
| [ ] | `CaseIterable` synthesis | ✅ | ★★ | R4 |
| [ ] | Nested types | ✅ | ★★ | R2 |

### 2b. Properties
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | Stored properties (let/var) | ✅ | ★ | R2 |
| [ ] | Computed properties (get/set) | ✅ | ★★ | R2 |
| [ ] | Read-only computed properties | ✅ | ★ | R2 |
| [ ] | Property observers `willSet`/`didSet` | ✅ | ★★ | R3 |
| [ ] | `lazy` stored properties | ✅ | ★★ | R3 |
| [ ] | Type properties `static`/`class` | ✅ | ★★ | R2 |
| [ ] | Property wrappers `@propertyWrapper` | ✅ | ★★★ | R5 |
| [ ] | Projected values `$wrapper` | ⚠️ | ★★★ | R5 |
| [ ] | Global & local variables (lazy globals) | ✅ | ★★ | R2 |

### 2c. Optionals
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | Optional type `T?` | ✅ | ★★ | R2 |
| [ ] | `if let` / `guard let` binding | ✅ | ★★ | R2 |
| [ ] | Shorthand `if let x` (Swift 5.7) | ✅ | ★ | R2 |
| [ ] | Forced unwrap `!` (trap on nil) | ✅ | ★ | R2 |
| [ ] | Optional chaining `?.` | ✅ | ★★★ | R2 |
| [ ] | Nil-coalescing `??` | ✅ | ★ | R2 |
| [ ] | Implicitly unwrapped optionals `T!` | ✅ | ★★ | R2 |
| [ ] | `Optional` pattern `case let x?` | ✅ | ★★ | R2 |

### 2d. Subscripts
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | Instance subscripts | ✅ | ★★ | R3 |
| [ ] | Type subscripts (`static subscript`) | ✅ | ★★ | R3 |
| [ ] | Subscript overloads / multi-param | ✅ | ★★ | R3 |

---

## Tier 3 — Reference Types & Memory (ARC)

| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | `class` declaration + reference semantics | ✅ | ★★★ | R3 |
| [ ] | **ARC retain/release** (refcount, no cycle GC) | ✅ | ★★★ | R3 |
| [ ] | Deterministic `deinit` at refcount 0 | ✅ | ★★ | R3 |
| [ ] | Inheritance + method/property override | ✅ | ★★★ | R3 |
| [ ] | `final` / `override` semantics | ✅ | ★★ | R3 |
| [ ] | Dynamic dispatch (vtables) | ✅ | ★★★ | R3 |
| [ ] | `super.` calls | ✅ | ★★ | R3 |
| [ ] | Designated / convenience initializers | ✅ | ★★★ | R3 |
| [ ] | Initializer delegation + 2-phase init | ✅ | ★★★★ | R3 |
| [ ] | `required` initializers | ✅ | ★★ | R3 |
| [ ] | Failable initializers `init?` / `init!` | ✅ | ★★ | R3 |
| [ ] | `weak` references (zeroing side table) | ✅ | ★★★ | R3 |
| [ ] | `unowned` references | ✅ | ★★ | R3 |
| [ ] | `unowned(unsafe)` | ⚠️ | ★★ | R3 |
| [ ] | Identity `===` `!==` | ✅ | ★ | R3 |
| [ ] | Type casting `is` / `as?` / `as!` / `as` | ✅ | ★★★ | R3 |
| [ ] | Downcasting in class hierarchies | ✅ | ★★★ | R3 |

### 3a. Closures
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | Closure expressions | ✅ | ★★★ | R3 |
| [ ] | Trailing closures (+ multiple) | ✅ | ★★ | R3 |
| [ ] | Shorthand args `$0 $1` | ✅ | ★★ | R3 |
| [ ] | Capture by reference (open/closed upvalues) | ✅ | ★★★ | R3 |
| [ ] | Capture lists `[weak self]` `[unowned]` | ✅ | ★★★ | R3 |
| [ ] | `@escaping` closures | ✅ | ★★★ | R3 |
| [ ] | `@autoclosure` | ✅ | ★★ | R3 |
| [ ] | Closures capturing `inout` | ✅ | ★★★ | R3 |

---

## Tier 4 — Abstraction: Protocols, Generics, Extensions

### 4a. Protocols
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | Protocol declaration (methods/props/init) | ✅ | ★★★ | R4 |
| [ ] | Conformance + **witness tables** (msf ConformanceTable) | ✅ | ★★★ | R4 |
| [ ] | Protocol inheritance | ✅ | ★★ | R4 |
| [ ] | Protocol composition `P & Q` | ✅ | ★★ | R4 |
| [ ] | Default implementations (in extensions) | ✅ | ★★★ | R4 |
| [ ] | Associated types (msf AssocTypeTable) | ✅ | ★★★ | R4 |
| [ ] | Protocol as type / existential `any P` | ✅ | ★★★ | R4 |
| [ ] | `Self` requirements | ✅ | ★★★ | R4 |
| [ ] | Protocol witness for operators | ✅ | ★★ | R4 |
| [ ] | Optional protocol requirements (`@objc optional`) | ⚠️ | ★★★ | R4+ |
| [ ] | Class-only protocols (`AnyObject`) | ✅ | ★★ | R4 |
| [ ] | Conditional conformance | ✅ | ★★★ | R4 |
| [ ] | Synthesized `Equatable`/`Hashable`/`Comparable` | ✅ | ★★★ | R4 |
| [ ] | Synthesized `Codable` (Encodable/Decodable) | ✅ | ★★★★ | R5 |

### 4b. Generics
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | Generic functions `<T>` | ✅ | ★★★ | R4 |
| [ ] | Generic types (struct/class/enum) | ✅ | ★★★ | R4 |
| [ ] | Type constraints `<T: Protocol>` | ✅ | ★★★ | R4 |
| [ ] | `where` clauses (msf `type_substitute`) | ✅ | ★★★ | R4 |
| [ ] | Associated-type constraints | ✅ | ★★★ | R4 |
| [ ] | Generic subscripts | ✅ | ★★ | R4 |
| [ ] | Monomorphization vs witness dispatch | ✅ | ★★★★ | R4 |
| [ ] | Contextual `where` on extensions | ✅ | ★★★ | R4 |
| [ ] | Parameter packs / variadic generics `each` | ⚠️ | ★★★★ | R6+ |
| [ ] | Integer generic parameters (`let N: Int`) | ⚠️ | ★★★ | R6+ |
| [ ] | `~Copyable` / `~Escapable` (suppressed constraints) | ✅ | ★★★★ | R6+ |

### 4c. Extensions
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | Extend struct/class/enum/protocol | ✅ | ★★ | R4 |
| [ ] | Add methods/computed props/inits/subscripts | ✅ | ★★ | R4 |
| [ ] | Add protocol conformance via extension | ✅ | ★★★ | R4 |
| [ ] | Conditional extensions (`where`) | ✅ | ★★★ | R4 |
| [ ] | Extensions on generic types | ✅ | ★★★ | R4 |

---

## Tier 5 — Error Handling & Resource Management

| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | `Error` protocol + custom errors | ✅ | ★★ | R5 |
| [ ] | `throws` functions | ✅ | ★★★ | R5 |
| [ ] | `throw` statement | ✅ | ★★ | R5 |
| [ ] | `do` / `catch` (+ pattern catches) | ✅ | ★★★ | R5 |
| [ ] | `try` / `try?` / `try!` | ✅ | ★★ | R5 |
| [ ] | `rethrows` | ✅ | ★★★ | R5 |
| [ ] | Typed throws `throws(E)` (Swift 6) | ⚠️ | ★★★ | R5 |
| [ ] | `defer` statements (LIFO on scope exit) | ✅ | ★★ | R5 |
| [ ] | Error propagation through call stack | ✅ | ★★★ | R5 |
| [ ] | `Result<Success, Failure>` (stdlib) | ✅ | ★★ | R5 |

---

## Tier 6 — Advanced Types & Expressions

| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | Opaque types `some P` (return) | ✅ | ★★★ | R4 |
| [ ] | Boxed/existential `any P` | ✅ | ★★★ | R4 |
| [ ] | Metatypes `T.self` / `.Type` / `.Protocol` | ✅ | ★★★ | R4 |
| [ ] | `type(of:)` dynamic type | ✅ | ★★ | R4 |
| [ ] | Key paths `\Root.path` | ✅ | ★★★ | R6+ |
| [ ] | Key-path expressions as functions | ⚠️ | ★★★ | R6+ |
| [ ] | `@dynamicMemberLookup` | ✅ | ★★★ | R6+ |
| [ ] | `@dynamicCallable` | ⚠️ | ★★★ | R6+ |
| [ ] | `#selector` / `#keyPath` | ⚠️ | ★★ | R6+ |
| [ ] | Self type | ✅ | ★★ | R4 |
| [ ] | Implicit member expr `.foo` | ✅ | ★★ | R2 |
| [ ] | `consume` / `borrow` operators (ownership) | ✅ | ★★★★ | R6+ |
| [ ] | `discard self` | ✅ | ★★★ | R6+ |

---

## Tier 7 — Concurrency

*Requires a scheduler/executor; tree-walker can't suspend the C stack — needs the
R6 bytecode VM (save/restore frame) per the design doc §13.*

| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | `async` functions | ✅ | ★★★★ | R6+ |
| [ ] | `await` expressions | ✅ | ★★★★ | R6+ |
| [ ] | `async let` | ⚠️ | ★★★★ | R6+ |
| [ ] | `Task` / `Task.detached` | ✅(stdlib) | ★★★★ | R6+ |
| [ ] | Task groups (`withTaskGroup`) | ✅(stdlib) | ★★★★ | R6+ |
| [ ] | Task cancellation | n/a | ★★★ | R6+ |
| [ ] | `actor` declarations + isolation | ✅ | ★★★★ | R6+ |
| [ ] | Actor reentrancy / serial executor | n/a | ★★★★ | R6+ |
| [ ] | `@MainActor` / global actors | ✅ | ★★★★ | R6+ |
| [ ] | `nonisolated` / `isolated` params | ✅ | ★★★ | R6+ |
| [ ] | `Sendable` checking | ✅ | ★★★ | R6+ |
| [ ] | `AsyncSequence` / `for await` | ⚠️ | ★★★★ | R6+ |
| [ ] | Continuations (`withCheckedContinuation`) | n/a | ★★★★ | R6+ |
| [ ] | Strict concurrency (Swift 6 mode) | ✅ | ★★★ | R6+ |

---

## Tier 8 — Macros & Metaprogramming (Swift 5.9+)

*Heavy: real macros run a separate compiler plugin (SwiftSyntax). A faithful runtime
needs a macro-expansion engine over the AST before evaluation.*

| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | Freestanding macros `#macro` | ⚠️ | ★★★★ | R6+ |
| [ ] | Attached macros `@Macro` | ⚠️ | ★★★★ | R6+ |
| [ ] | Macro declarations | ✅(AST_MACRO_DECL) | ★★★★ | R6+ |
| [ ] | Built-in `#file`/`#line`/`#function`/`#column` | ⚠️ | ★★ | R5 |
| [ ] | `#warning` / `#error` | ⚠️ | ★ | R1 |
| [ ] | `@freestanding` / `@attached` roles | ⚠️ | ★★★★ | R6+ |
| [ ] | `@resultBuilder` (DSL transform) | ⚠️ | ★★★★ | R6+ |
| [ ] | Result-builder method synthesis | ❌ | ★★★★ | R6+ |

---

## Tier 9 — Attributes, Access Control, Operators, Directives

### 9a. Access control
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | `open` `public` `internal` `fileprivate` `private` | ✅ | ★★ | R4 |
| [ ] | `package` access level | ✅ | ★★ | R4 |
| [ ] | Access on setters `private(set)` | ✅ | ★★ | R4 |
| [ ] | Module boundaries / `import` | ✅ | ★★★ | R5 |

### 9b. Custom operators
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | `prefix`/`infix`/`postfix` operator decls | ✅ | ★★★ | R4 |
| [ ] | `precedencegroup` (+ `higherThan`/`assoc`) | ✅ | ★★★ | R4 |
| [ ] | Operator method implementations | ✅ | ★★ | R4 |
| [ ] | Operator overloading | ✅ | ★★ | R4 |

### 9c. Attributes (34 declaration/type attributes)
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | `@available` (+ availability conditions `#available`) | ✅ | ★★ | R5 |
| [ ] | `@objc` / `@nonobjc` / `@objcMembers` | ⚠️ | ★★ | R6+ |
| [ ] | `@main` entry point | ✅ | ★★ | R5 |
| [ ] | `@frozen` / `@inlinable` / `@usableFromInline` | ⚠️ | ★ | R6+ |
| [ ] | `@inline` / `@_optimize` (perf hints) | ⚠️ | ★ | R6+ |
| [ ] | `@discardableResult` | ✅ | ★ | R1 |
| [ ] | `@propertyWrapper` | ✅ | ★★★ | R5 |
| [ ] | `@resultBuilder` | ⚠️ | ★★★★ | R6+ |
| [ ] | `@globalActor` | ⚠️ | ★★★★ | R6+ |
| [ ] | `@Sendable` | ✅ | ★★ | R6+ |
| [ ] | `@autoclosure` / `@escaping` / `@convention` | ✅ | ★★ | R3 |
| [ ] | `@dynamicMemberLookup` / `@dynamicCallable` | ✅/⚠️ | ★★★ | R6+ |
| [ ] | `@preconcurrency` / `@unchecked` | ✅ | ★★ | R6+ |
| [ ] | `@NSCopying` / `@NSManaged` / IB attrs | ⚠️ | ★★ | R6+ |
| [ ] | `@backDeployed` / `@_specialize` | ⚠️ | ★ | R6+ |
| [ ] | `@warn_unqualified_access` / misc diagnostics | ⚠️ | ★ | R6+ |

### 9d. Compiler control / directives
| ✓ | Feature | FE | RT | Phase |
|---|---|----|----|-------|
| [ ] | Conditional compilation `#if`/`#elseif`/`#else`/`#endif` | ⚠️ | ★★ | R5 |
| [ ] | `#if` platform/arch/compiler/`canImport`/`swift()` | ⚠️ | ★★ | R5 |
| [ ] | `#sourceLocation` line control | ⚠️ | ★ | R6+ |
| [ ] | `#available` / `#unavailable` conditions | ✅ | ★★ | R5 |

---

## Tier 10 — Standard Library Surface (behaviour, not syntax)

*msf provides the type **shapes** (baked vocab) but **no behaviour** — this is the
biggest sustained effort. Scope deliberately.*

### 10a. Core values
| ✓ | Feature | RT | Phase |
|---|---|----|-------|
| [ ] | `Int`/`UInt` family (all widths) + overflow ops | ★★ | R0 |
| [ ] | `Float`/`Double` + math | ★★ | R0 |
| [ ] | `Bool` | ★ | R0 |
| [ ] | `String` (UTF-8, NFC, views) + `Character` | ★★★★ | R1 |
| [ ] | `Substring` | ★★★ | R2 |
| [ ] | `Optional<Wrapped>` | ★★ | R2 |
| [ ] | `Range`/`ClosedRange`/`Stride` | ★★ | R1 |
| [ ] | Tuples | ★★ | R1 |

### 10b. Collections (value semantics + CoW)
| ✓ | Feature | RT | Phase |
|---|---|----|-------|
| [ ] | `Array<Element>` + CoW | ★★★★ | R1 |
| [ ] | `Dictionary<Key,Value>` + CoW | ★★★★ | R2 |
| [ ] | `Set<Element>` + CoW | ★★★ | R2 |
| [ ] | `ContiguousArray` / `ArraySlice` | ★★★ | R4 |
| [ ] | `isKnownUniquelyReferenced` (CoW correctness) | ★★★ | R3 |

### 10c. Protocols that drive the language
| ✓ | Feature | RT | Phase |
|---|---|----|-------|
| [ ] | `Equatable` / `Hashable` / `Comparable` | ★★★ | R4 |
| [ ] | `Sequence` / `IteratorProtocol` | ★★★ | R4 |
| [ ] | `Collection` / `BidirectionalCollection` / `RandomAccess` | ★★★★ | R4 |
| [ ] | `RangeReplaceableCollection` | ★★★ | R4 |
| [ ] | `ExpressibleBy*Literal` (literal conversion) | ★★★ | R2 |
| [ ] | `CustomStringConvertible` / `Debug…` | ★★ | R2 |
| [ ] | `RawRepresentable` / `CaseIterable` | ★★ | R2 |
| [ ] | `Codable` / `Encodable` / `Decodable` | ★★★★ | R5 |
| [ ] | `Identifiable` | ★ | R4 |
| [ ] | `Sendable` | ★★ | R6+ |
| [ ] | `AsyncSequence` / `AsyncIteratorProtocol` | ★★★★ | R6+ |

### 10d. Functions & utilities
| ✓ | Feature | RT | Phase |
|---|---|----|-------|
| [ ] | `print` / `debugPrint` / `dump` | ★ | R0 |
| [ ] | `map`/`filter`/`reduce`/`flatMap`/`compactMap`/`sorted`/… | ★★★ | R4 |
| [ ] | `assert`/`precondition`/`fatalError`/`assertionFailure` | ★★ | R1 |
| [ ] | `min`/`max`/`abs`/`stride`/`zip`/`swap` | ★★ | R2 |
| [ ] | `Result` | ★★ | R5 |
| [ ] | `MemoryLayout` | ★★ | R6+ |
| [ ] | `Unsafe*Pointer` family | ★★★★ | R6+ |

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
- [ ] Verify raw-string / extended-delimiter lexing edge cases
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
