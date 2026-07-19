# Plan — Swift Testing support (`tswift test`)

**Status:** proposed (research only; no implementation yet)
**Date:** 2026-07-19
**Reference:** [swift-testing](https://github.com/swiftlang/swift-testing) (Swift 6 / Xcode 16+), Apple Testing docs, local `swift test` console capture (Swift 6.4)
**Related:**
- [ADR-0017](../adr/0017-multi-file-program-input.md) — multi-file program input (concatenation + `FileSpan`)
- [AGENTS.md](../../AGENTS.md) / [CONTEXT.md](../../CONTEXT.md) — architecture vocabulary
- Feature checklist macros tier — freestanding macros are builtins today, not a general expansion engine
- Precedent: `#Predicate` via `Interpreter::register_macro` (`tswift-swiftdata`)

---

## 0. Decision summary (who owns what)

| Concern | Owner | Not |
|---|---|---|
| Test discovery (`@Test` / `@Suite`) | New **runner** walk over typed AST after `analyze_program` | Compiler plugin / symbol mangling / binary reflection |
| `#expect` / `#require` | **Interpreter freestanding macros** (`register_macro`), same seam as `#Predicate` | Sema-time macro expansion engine |
| Failure detail (expr text + operands) | Macro handlers + optional AST pretty-printer | Full SwiftSyntax macro expansion |
| Suite lifecycle (fresh instance per test) | Runner: construct suite type, call method | XCTestCase inheritance |
| CLI surface | `tswift test` in `tswift-cli` | Reusing `tswift run` + fake `main` |
| Framework symbols (`import Testing`) | New crate **`tswift-testing`** installed under module `"Testing"` | Shipping Apple’s package; XCTest |

**Core bet:** Swift Testing’s user-facing surface is *attributes + two freestanding macros*. The frontend already parses both (`@Test`/`@Suite` as `Attribute` children; `#expect`/`#require` as `CompilerDirective`). We do not need a macro-expansion engine for a useful subset — we need discovery + a runner + two macro handlers + a CLI.

---

## 1. Scope — Swift Testing surface

Grounded in real-world usage (WWDC24 Swift Testing, Apple docs, migration playbooks) and ranked for tswift’s interpreter model.

### 1.1 Must-have (MVP — slices A–B)

| Feature | Notes |
|---|---|
| `@Test` free functions | Top-level `FuncDecl` with `Attribute "Test"` |
| `@Test` methods in types | Methods on `struct`/`class` (with or without `@Suite`) |
| Display name | `@Test("…")` — first unlabeled string arg on the attribute |
| `#expect(expr)` | Soft fail: record issue, **continue** the test body |
| `#require(expr)` / `try #require(optional)` | Hard fail: abort test; optional unwrap returns value |
| Failure detail (v1) | Expression source spelling + operand values for common binary ops (`==`, `!=`, `<`, …) and unary bool |
| Throwing tests | `func f() throws` — uncaught throw → test failure (1 issue) |
| Async tests | `func f() async` / `async throws` — cooperative executor already exists |
| Exit codes | `0` all pass (or only skips); `1` any failure/issue; compile/load errors also non-zero |
| CLI | `tswift test <files\|dir\|package> [--filter <name>]` |

### 1.2 Later (slice C and follow-ups)

| Feature | Priority within “later” | Notes |
|---|---|---|
| `@Suite` display name + nesting | High | **Landed (slice C)** — nested suite types; suite traits inherited by children |
| Traits: `.disabled("…")` | High | **Landed (slice C)** — skip with reason (very common) |
| Traits: `.enabled(if:)` | High | **Landed (slice C)** — evaluate condition once at run start |
| `@Test(arguments:)` parameterized | High | **Landed (slice C)** — one case per element; cartesian multi-collection; `zip` |
| `Issue.record(…)` / `Issue.record(Comment, …)` | Medium | **Landed (slice C)** — `Issue.record(_: String)` soft failure |
| Traits: `.tags(…)` + filter by tag | Medium | **Landed (slice F)** — `.tags(.fast)` read structurally (tag identity by name), inherited by suite members; CLI `--filter tag:<name>` selects by tag |
| `#expect(throws:)` / `throws: Never.self` | Medium | **Landed (slice E)** — closure-form overloads: type match, `Never.self`, and error-instance equality; `try #require(throws:)` returns the thrown error |
| Traits: `.bug(…)` | Low | **Landed (slice F)** — `.bug("url"/id)` surfaced as a `bug:` annotation on a failing test |
| Traits: `.serialized` | Low | Serial is **default** in tswift v1 anyway |
| Traits: `.timeLimit` | Low | **Landed (slice F)** — **soft** check: measures duration, records an issue when `.minutes(n)`/`.seconds(n)` exceeded; no hard kill (documented divergence) |
| `confirmation { }` | Low | Depends on async wait helpers |
| `withKnownIssue` | Low | **Landed (slice F)** — body failure/throw is an expected *known* issue (reported distinctly, does not fail the run); a body that passes cleanly is itself a failure |
| `CustomTestStringConvertible` | Low | **Deferred** — parameterized case labels use the argument node's source spelling, not an evaluated `testDescription`; params expansion is structural (nodes are never evaluated), so honouring the conformance would need per-element evaluation + a conformance seam. Reopen when parameterized labels need runtime value rendering |
| Parallel test execution | Low | Real library default-parallel; tswift v1 is **serial** (one interpreter) |
| Nested suite trait inheritance | Medium | **Landed (slice C)** |
| `class` suites with `deinit` teardown | Medium | **Landed (slice F)** — the suite driver binds the instance to a `do`-block local (`do { var __suite = Suite(); … }`) so a `class` suite's `deinit` runs deterministically after each test (expression temporaries are not ARC-released, so the bare-temporary form never fired `deinit`) |

### 1.3 Document-unsupported (explicit non-goals for this plan)

| Feature | Why |
|---|---|
| **XCTest** / dual-framework coexistence | Separate framework; out of scope |
| UI tests (`XCUIApplication`), performance (`XCTMetric`) | Host/process model tswift does not have |
| Full macro expansion engine | Feature checklist R6+; not required for Testing MVP |
| Source-accurate multi-file *runtime* stack traces for every trap | ADR-0017 known degradation; runner remaps issue locations via `FileSpan` for `#expect` lines |
| True module isolation / `@testable` visibility | Concatenation flattens one compilation unit (ADR-0017); treat SUT + tests as one unit |
| Compiler-plugin `#expect` diagnostics (“always fail” warnings) | Requires type-aware constant folding in sema |
| Xcode Test Navigator / event stream SPI | CLI + optional JSON reporter later |
| Wasm/browser `tswift test` | CLI-first; wasm can reuse runner later |

### 1.4 Ranking rationale (usage → priority)

From real suites and migration guides:

1. People write **free `@Test` + `#expect`** first — must work day one.
2. **`#require` for unwrap** is the second most-cited migration win over `XCTUnwrap`.
3. **`@Suite` + `init` isolation** is the structural pattern once tests grow.
4. **`.disabled` / `.enabled(if:)`** appear immediately in CI hygiene.
5. **Parameterized tests** replace boilerplate and are heavily marketed — but need collection eval + case expansion; land after runner is solid.
6. Tags, known issues, confirmations, throws-matchers are power features — after the core loop is green.

---

## 2. Discovery, execution model, exit codes

### 2.1 Verified frontend facts (grounding)

AST dump of representative sources (via `tswift dump`, 2026-07-19):

```text
@Test("named")
func freeNamed() { #expect(1 == 2) }

→ FuncDecl "freeNamed"
     Block → CompilerDirective "expect" → BinaryExpr "==" …
     Attribute "Test" → StringLiteral "\"named\""

@Suite("My Suite")
struct NamedSuite {
  @Test func t() throws { #expect(true) }
}

→ StructDecl "NamedSuite"
     FuncDecl "t" → … Attribute "Test"
     Attribute "Suite" → StringLiteral "\"My Suite\""

@Test(arguments: [1, 2]) func p(x: Int) { … }
→ Attribute "Test" → ArrayLiteral "[" …  (label "arguments" on child)

@Test(.disabled("x")) func skip() {}
→ Attribute "Test" → CallExpr → MemberExpr "disabled" + StringLiteral
```

Implications:

- Attribute text is **without** `@` (`"Test"`, `"Suite"`) — matches existing `@main` / `@Model` consumers.
- Macro text is **without** `#` on the frontend `Node::text()` path (`"expect"`, `"require"`) — matches `eval_macro` keying.
- Attribute arguments are already full expression subtrees (labels via `arg_label`) — traits and `arguments:` are inspectable without parser work.
- `#expect` / `#require` children are ordinary typed expressions — ready for evaluation.

### 2.2 Multi-file / package input (ADR-0017)

Reuse the same program-input path as `tswift run`:

1. Expand paths → ordered `Vec<SourceFile>` (`collect_source_files` / project loader).
2. `Analysis::analyze_program(&files)` — single concatenated unit + `FileSpan` table.
3. Abort on compile errors (same diagnostic rendering as `run`).
4. Discover tests on the typed root; remap each test’s `Node::line()` through `FileSpan` → `(path, local_line)` for reports.

**Package / `.testTarget` gap (verified):** `tswift_frontend::project::load_program` currently rejects `TargetKind::Other("testTarget")` with `UnsupportedTargetKind`. Slice B must:

- Teach the loader a **test-mode** path (e.g. `load_test_program` or `load_program(..., ProgramKind::Test)`) that accepts `.testTarget`.
- Default target selection for `tswift test <pkg>`: all `.testTarget`s (run sequentially), or `--target Name` for one.
- **Dependencies:** real SwiftPM links test target → library target. tswift has no link step. v1: concatenate test target sources **plus** each dependency target’s sources into one unit (same flat-module model as multi-file today). Document that `import` / `@testable import` of those modules is best-effort (import gate records the name; symbols live in one env).

### 2.3 Discovery algorithm

After successful analysis + hoist/register (interpreter `eval` of root decls so types/functions exist):

```
discovered = []

for each top-level decl in analysis.root():
  if FuncDecl with Attribute "Test":
    push FreeTest { name, display_name, attrs, node, file_span }

  if StructDecl | ClassDecl | ActorDecl:
    let suite_attr = Attribute "Suite" | implicit if any member has @Test
    if suite_attr or has @Test members:
      for each member FuncDecl with Attribute "Test":
        push SuiteTest { suite_type, suite_display, test_name, … }
      // nested types: recurse (later)

Apply --filter: substring match on fully-qualified id
  free:   "addition()" or display name
  suite:  "MathSuite/pass()" or "MathSuite.pass"
```

**Implicit suites:** A type with `@Test` methods is a suite even without `@Suite` (matches Apple). `@Suite` only adds display name + traits.

**Not discovered:** nested functions, local closures, protocol requirements without bodies, `#if`-disabled decls (respect existing `expand_directives` / active branch if the walker uses the same helper as nominal registration).

### 2.4 Execution model

```
install stdlib + foundation + … + tswift_testing::install(interp)
eval root  // hoist types, free funcs, globals (no top-level tests auto-run)

for test in discovered (stable order: file path, line):
  if skipped by trait → record skip; continue
  push TestContext { issues: [], name, … } onto interpreter
  start timer
  match test:
    FreeTest → call_function(test_fn, args_for_parameterized?)
    SuiteTest →
      instance = construct suite type (default init / throws init)
      call method on instance
  on Throw / Trap / RequireFailure → convert to Issue, abort body
  pop TestContext
  report pass | fail | skip + duration

print summary; exit
```

**Isolation:** Fresh suite **instance per test** (Apple’s model). Free tests share only process-global state (same as real Swift Testing in one process).

**Serial v1:** One interpreter, sequential tests. Parallelism is a later opt-in (would need multiple interpreters or re-entrancy policy). Document the divergence from Apple’s default-parallel.

**No `@main`:** Test programs must not require top-level executable code. `main.swift` in a test target is unusual; if present, either ignore top-level statements or error clearly (“test targets must not use top-level code”). Prefer: discovery-only entry — never call `@main` during `tswift test`.

### 2.5 Exit codes

| Code | Meaning |
|---|---|
| `0` | Analysis OK; every non-skipped test passed (zero issues) |
| `1` | Analysis OK; ≥1 failed test or uncaught runner error during a test |
| `1` (also) | Zero tests discovered? **Recommend non-zero** with message `error: no tests found` (avoids silent CI green) — verify against `swift test` empty-target behavior before locking (open question Q3) |
| `1` | Compile/load/IO failure (same as `tswift run` failure path) |

No separate “usage error” code required for v1.

---

## 3. `#expect` / `#require` implementation

### 3.1 Pattern to copy: freestanding macro builtins

Existing seam (`crates/tswift-core`):

```rust
// stdlib.rs
pub type MacroFn = fn(&mut dyn StdContext, &Node<'static>) -> StdResult;

// Interpreter::register_macro("Predicate", handler)
// eval_macro: strip #, look up handler, else builtins (#file/#line/…)
```

`#Predicate` is **not** expanded to Swift AST. The handler inspects the `CompilerDirective` node’s children (type args + closure) and returns a value.

**Decision:** Implement `#expect` and `#require` the same way inside `tswift-testing`:

```rust
interp.begin_module("Testing");
interp.register_macro("expect", expect_macro);
interp.register_macro("require", require_macro);
// optional: Issue.record as free/static API
interp.end_module();
```

Requires `import Testing` under strict import-gating **or** auto-import `Testing` for the `tswift test` command only (recommended for ergonomics; document that `tswift run` does not install Testing unless imported and installed).

### 3.2 Why not full macro expansion

Real Swift Testing macros rewrite:

```swift
#expect(a == b)
// roughly →
Testing.__checkBinaryOp(a, b, "==", sourceLocation, { a }, { b }, "a == b")
```

That needs a SwiftSyntax plugin pipeline. tswift’s checklist marks the expansion engine R6+. For Testing we only need **runtime check semantics + good messages**, which the AST already carries:

- Structure for operand splits (`BinaryExpr` children)
- Types from sema annotations
- Line/col for location

### 3.3 `#expect` semantics

```
expect_macro(ctx, node):
  require TestContext is active  // else runtime error: #expect outside test
  expr = first meaningful child expression
  // Overloads later: throws:, Comment, …
  result = eval_with_operand_capture(expr)
  match result:
    Bool(true)  → Ok(Void)
    Bool(false) → record Issue { kind: ExpectationFailed, expr_text, operands, loc }; Ok(Void)
    other       → type error / trap “#expect requires Bool”
  // never throws for ordinary false
```

**Continue-on-failure:** issues append to `TestContext`; body keeps running. Runner marks test failed if `issues.nonEmpty` at end.

### 3.4 `#require` semantics

```
require_macro(ctx, node):
  expr = child
  v = eval(expr)
  if is_optional_nil(v):
    record Issue { RequireFailed, … }
    return Err(RequireAbort)  // special signal → abort test only
  if is_bool(v) && v == false:
    record Issue; return Err(RequireAbort)
  return Ok(unwrap_if_optional(v))  // non-optional values pass through
```

`try #require(…)` already parses as `TryExpr` wrapping `CompilerDirective` (verified). Throwing path: map `RequireAbort` through try like a throw, or use a dedicated `Signal` that the test runner catches even without `try` (prefer: **always abort the test** whether or not `try` is written, matching “hard check”; if `try` is present it still typechecks as throwing in real Swift — tswift can accept both).

### 3.5 Capturing expression text and operands

**Expression text (v1 — good enough):**

1. **AST pretty-printer** for common kinds: `BinaryExpr`, `PrefixExpr`, `CallExpr`, `MemberExpr`, `IdentExpr`, literals, `SubscriptExpr`, `TryExpr`/`AwaitExpr` wrappers. Emit source-like string (`add(1, 1) == 3`).
2. **Fallback:** `"<expression>"` + location if unprintable.

**Why not slice original source:** nodes store start `line`/`col` only (no end offset in the arena API used by the runtime). Multi-file concatenation further complicates raw slicing. Pretty-print is self-contained and works across files.

**Operand values (v1):**

For `BinaryExpr` with comparison/equality ops:

```
lhs_v = eval(lhs); rhs_v = eval(rhs)
bool_v = apply_op(lhs_v, op, rhs_v)
on failure:
  primary:  "{pretty} → false"
  detail:   "  {pretty_lhs} → {describe(lhs_v)}"
            "  {pretty_rhs} → {describe(rhs_v)}"  // when rhs not literal-only optional
```

Reuse `SwiftValue` `Display` / `describe_with_type` (`value.rs`) for operand formatting.

For bare `#expect(flag)`: show `flag → false` when `flag` is an `IdentExpr` / `MemberExpr`.

**v2 upgrades (not blocking):** subexpression tree dumps matching Xcode’s multi-line caret; operator-specific messages; `#expect(throws:)`.

### 3.6 Target console shape (captured from real `swift test`)

```text
􀟈  Test run started.
􀟈  Suite MathSuite started.
􀟈  Test addition() started.
􀙟  Test skipMe() skipped: "known flaky"
􁁛  Test pass() passed after 0.001 seconds.
􁁛  Suite MathSuite passed after 0.001 seconds.
􀢄  Test addition() recorded an issue at BasicTests.swift:6:3: Expectation failed: add(1, 1) == 3
􀄵  add(1, 1) == 3 → false
􀄵    add(1, 1) → 2
􀢄  Test addition() failed after 0.001 seconds with 1 issue.
􀢄  Test run with 3 tests in 1 suite failed after 0.001 seconds with 1 issue.
```

**tswift v1 output:** same *structure and wording*, with **ASCII/emoji-safe status markers** (e.g. `✔` / `✘` / `↷` or `[PASS]`/`[FAIL]`/`[SKIP]`) so CI logs on non-Apple terminals stay readable. Optional `--style apple` later for SF Symbol codepoints.

---

## 4. CLI design

### 4.1 Interface

```text
tswift test <file.swift> [more.swift ...]
tswift test <dir>                    # all *.swift (flat) or package if Package.swift
tswift test <dir-with-Package.swift> [--target <testTargetName>]
tswift test … [--filter <substring>]
tswift test … [--allow-network]      # same host caps as run, for tests that hit URLSession
```

Wire into `crates/tswift-cli/src/main.rs` next to `run` / `dump` / `symbols`.

Reuse:

- `collect_source_files` / project collection (extend for test targets)
- `render_diagnostic` for compile failures
- Host installs (defaults, fs, db, foundation, …) identical to `run` so tests can exercise the same surface

Do **not** route through `interp.run()`’s `@main` path. New entry:

```rust
// conceptual
let analysis = Analysis::analyze_program(&files)?;
let mut interp = Interpreter::new(...);
install_all(&mut interp);
tswift_testing::install(&mut interp);
let report = tswift_testing::run_tests(&mut interp, analysis, RunOptions { filter, … })?;
print_report(&report);
ExitCode::from(report)
```

### 4.2 Filter

v1: case-sensitive **substring** on the test’s display id:

- `addition`
- `MathSuite`
- `MathSuite/pass`
- display name string if set

Later: tag filters (`.fast`), regex, exclude.

### 4.3 Summary line

Mirror Apple:

```text
Test run with {n} tests in {s} suites passed after {t} seconds.
Test run with {n} tests in {s} suites failed after {t} seconds with {i} issues.
```

Include skipped count when non-zero.

### 4.4 What is *not* in v1 CLI

- JSON / xUnit reporters (add when CI consumers need them)
- `--parallel`
- Watch mode
- XCTest mixed runners

---

## 5. Crate / module layout

```text
crates/tswift-testing/          # new, mirrors tswift-swiftdata / tswift-eventkit
  src/
    lib.rs                      # install(), run_tests(), public types
    discover.rs                 # AST walk → Vec<TestCase>
    expect.rs                   # #expect / #require macros
    report.rs                   # console formatting + RunReport
    traits.rs                   # parse Attribute args → Trait enum (slice C)
    params.rs                   # arguments: expansion (slice C)

crates/tswift-cli/src/main.rs   # `test` subcommand (slice B)
crates/tswift-frontend/project  # load test targets (slice B)
```

**Keep `tswift-core` free of Testing knowledge** except:

- Optional tiny seam: `TestContext` storage on the interpreter **or** a callback/context slot on `StdContext` for “record issue / abort test” (preferred: trait methods on `StdContext` so macros stay in `tswift-testing`).

If `StdContext` must grow:

```rust
fn test_record_issue(&mut self, issue: TestIssue) { … default: err not in test }
fn test_abort(&mut self) -> StdError { … }
```

Default implementations error when not under a runner — preserves `tswift run` behavior.

**Cargo:** workspace member; no new crates.io deps (use existing time APIs / `std::time::Instant`).

---

## 6. Slice plan (stacked PRs, each &lt; ~1000 LOC)

TDD each slice: **write a failing test first**, then implement.

### Slice A — Core runner + `@Test` + `#expect` / `#require`

**Goal:** Library-level API runs free tests and reports issues; no CLI yet.

| Deliverable | Detail |
|---|---|
| `tswift-testing` crate | `install`, `discover`, `run_tests` |
| Free `@Test` discovery | Top-level only |
| `#expect` / `#require` macros | Bool + optional unwrap; binary operand capture for `==`/`!=`/`</>/…` |
| Pretty-printer v1 | Common expr kinds |
| `TestContext` | Soft issues + hard abort |
| Throwing / async test bodies | Via existing call/eval paths |
| Unit/integration tests | In-crate: analyze fixture strings, run, assert `RunReport` |

**TDD first tests:**

1. `expect_true_passes` — `@Test func t() { #expect(1 + 1 == 2) }` → 1 pass  
2. `expect_false_records_issue` — `#expect(1 == 2)` → fail, message contains `1 == 2` and operands  
3. `expect_continues_after_failure` — two `#expect(false)` → 2 issues, one failed test  
4. `require_aborts` — `#require(false)` then `#expect(false)` → 1 issue only  
5. `require_unwraps` — `let x = try #require(Optional.some(5) as Int?)` + `#expect(x == 5)`  
6. `throwing_test_failure` — `throw` without catch → failed test  

**Out of slice A:** CLI, `@Suite`, traits, parameterized, package targets.

**LOC budget:** crate scaffold + discover + macros + report structs ≈ 600–900.

---

### Slice B — CLI `tswift test`

**Goal:** Users run tests from the command line on files/dirs/packages.

| Deliverable | Detail |
|---|---|
| `tswift test` subcommand | Path collection + install stack + `run_tests` |
| Console reporter | Apple-shaped summary (ASCII markers) |
| `--filter` | Substring |
| Project loader | Accept `.testTarget`; default all test targets; concatenate dependency sources |
| Golden CLI tests | `crates/tswift-cli/tests/…` fixtures: pass, fail, filter, compile error |

**TDD first tests:**

1. Fixture dir with one passing test → exit 0, stdout contains `passed`  
2. Failing `#expect` → exit 1, issue line present  
3. `--filter` excludes non-matching tests  
4. Package with `.testTarget` loads (no `UnsupportedTargetKind`)  
5. Syntax error → non-zero, no “Test run started”  

**LOC budget:** CLI wiring + project changes + goldens ≈ 400–800.

---

### Slice C — Suites, traits, parameterized — **landed**

**Goal:** Cover the organizational surface most real suites use.

**Status (landed):**

- `@Suite` display names surface in the reported label; nested suite types are
  discovered recursively (`Outer.Inner()` construction, `Outer/Inner/b()` id).
- Traits `.disabled("reason")` and `.enabled(if: cond)` skip a test (reason
  shown, skip never fails the run); suite-level traits are inherited by every
  member, including nested suites.
- `@Test(arguments:)` expands structurally: a single array literal, the
  cartesian product of several array literals, or element-wise `zip(a, b)`.
  Each case runs independently with its argument value in the label
  (`div(x:) - 4`); a failing case fails only itself.
- `Issue.record(_: String)` records a manual soft failure (Testing-module
  static; attributed to the test's declaration line, no per-call location).
- CLI renders skip lines with reason + a skip count, and one line per
  parameterized case.

**Deferred out of Slice C — now landed in Slice E:**

- `#expect(throws:)` closure form: **landed (slice E)**. The parser now accepts
  keyword argument labels (`throws:`, `for:`, `in:`, …) and a trailing closure
  on a freestanding-macro directive, so `#expect(throws: E.self) { … }` reaches
  the macro handler. Implemented forms: `throws: E.self` (thrown type match),
  `throws: Never.self` (must not throw), `throws: instance` (error equality),
  and `try #require(throws:)` (hard-abort on mismatch, returns the thrown error
  on success). Works with async closures. A trap inside the closure is not a
  throw and still fails the test.

**Still deferred out of Slice C:**

- Argument values in parameterized labels are the source spelling of each
  element node (not an evaluated `CustomTestStringConvertible` rendering);
  fine for the common literal case.
- Tag filtering / `.tags` remains name-based only.

| Deliverable | Detail |
|---|---|
| `@Suite` / implicit suites | Fresh instance per test; `init` throws → fail that test |
| Nested suite types | One level or full recurse |
| Traits | `.disabled`, `.enabled(if:)` parsing from Attribute args |
| Parameterized | `@Test(arguments:)` — evaluate collection, expand cases; multi-arg cartesian; document `zip` if missing |
| `Issue.record` | Soft issue API |
| Filter | Still name-based; tags optional if cheap |

**TDD first tests:**

1. Suite method runs on isolated instance (counter field does not leak across tests)  
2. `.disabled("reason")` → skipped line in report, exit 0 if only skips + passes  
3. `.enabled(if: false)` → skip  
4. `arguments: [1, 2, 3]` → 3 cases; one failing case fails only that case  
5. Display names on suite and test appear in output  

**LOC budget:** traits + params + suite construct ≈ 700–1000; split C1 (suites+disabled) / C2 (params) if needed.

---

### Slice D — Website / docs

**Goal:** Document the feature for users and agents.

| Deliverable | Detail |
|---|---|
| Website status page | New section or row under language/tooling: `tswift test`, Testing subset table |
| README / how-to | Minimal “write a test / run it” |
| Feature checklist | Tick freestanding macro subset for `#expect`/`#require` *as builtins*; note not general macros |
| This plan → status `partial`/`landed` as slices merge |

**TDD:** `website` build green; no runtime tests required.

**LOC budget:** MDX + checklist ≈ 200–400.

---

### Slice F — Remaining “later” features — **landed**

**Goal:** Cover the highest-value power features left in the §1.2 table.

**Status (landed):**

- **Tags:** `.tags(.fast, .slow)` parsed structurally (tag identity by name;
  no runtime `Tag` value is constructed — the attribute is never evaluated),
  inherited by suite members like every trait. CLI `--filter tag:<name>`
  selects by an exact tag name; any other `--filter` needle stays a substring
  match.
- **`withKnownIssue("reason") { … }`:** a body that records an issue or throws
  is an *expected* (known) failure — reported distinctly (`◇ … recorded a
  known issue`) and does **not** fail the run. A body that instead completes
  cleanly is itself a real failure (“Known issue was not recorded”). A runtime
  trap inside the body is not an expected failure and still fails the test.
- **`.bug("url"/id)`:** a report-only annotation; surfaced as a `bug:` line
  under a failing test.
- **`.timeLimit(.minutes(n)` / `.seconds(n))`:** a **soft** check — the runner
  measures each test's duration and records an issue when the limit is
  exceeded, but never hard-kills the test. **Divergence from Apple:** real
  Swift Testing enforces the limit by cancelling the test; tswift has no host
  timer policy, so this is measure-and-report only.
- **`class` suite `deinit` teardown:** the suite driver binds the instance to a
  `do`-block local (`do { var __suite = Suite(); try await __suite.m() }`) so a
  `class` suite's `deinit` runs deterministically after each test. (Expression
  temporaries are not ARC-released by the interpreter, so the previous
  bare-temporary `Suite().m()` form never fired `deinit`.)

**Deferred out of Slice F:**

- **`CustomTestStringConvertible`:** parameterized case labels use each argument
  node's source spelling, not an evaluated `testDescription`. Params expansion
  is structural (element nodes are never evaluated), so honouring the
  conformance would need per-element evaluation plus a conformance seam —
  reopened when labels need runtime value rendering.
- **`confirmation { }`** and **parallel execution** remain deferred (async wait
  helpers / multi-interpreter policy).

**LOC budget:** traits + known-issue session + driver change + goldens ≈ 500.

---

### Slice G — List/select tests as a platform-neutral seam — **landed**

**Goal:** Let a host (CLI, web playground, iOS) enumerate the tests in a
program and run an explicit subset, without a bespoke integration per host.

**Status (landed):**

- **Library seam** (`tswift-testing`, single source of truth):
  - `list_tests(files) -> Vec<TestDescriptor>` — discovery **without running**
    (id, display name, suite path, file/line, tags, parameterized case count,
    static `.disabled` skip status/reason). A `.enabled(if:)` condition needs
    the program run, so it is *not* reflected in a descriptor's `skipped`
    field — documented.
  - `RunOptions` gains `ids: Option<Vec<String>>`: exact canonical-id
    selection, distinct from the substring/tag `filter`. A base test id runs
    all of a parameterized test's cases; an exact case id (`p() - 2`, argument
    value suffixed — the base id carries no parameter labels) runs one. An id
    matching no discovered test is an error listing the unknown ids, never a
    silent zero-tests success. `ids` takes precedence over `filter`.
  - `wire.rs` serializes descriptors + `RunReport` to JSON and parses run
    options, using the hand-rolled `tswift_core::json` layer (the workspace
    avoids `serde` for offline builds — the plan's "serde" note is superseded).
- **CLI:** `tswift test <inputs> --list [--json]` prints the list (human table
  or JSON); `tswift test <inputs> --test <id>` (repeatable) runs exactly those
  ids. `--test` and `--filter` are mutually exclusive (usage error on both).
- **Wasm:** `listTests(filesJson)` / `runTests(filesJson, optionsJson)`.
- **FFI:** `tswift_list_tests(module_json)` /
  `tswift_run_tests(module_json, options_json)` (stateless, following the
  `tswift_list_symbols` convention; header + ABI drift-guard updated).

**Out of Slice G:** the web playground's test-runner *UI* itself (the wasm/FFI
exports exist, but wiring a UI is separate).

**LOC budget:** descriptor + wire + selection + CLI/wasm/FFI wiring + tests ≈ 900.

---

### Suggested stack order

```text
A (library) → B (CLI) → C (suites/traits/params) → D (website) → E (throws) → F (tags/known-issue/bug/timeLimit/deinit)
```

A can merge alone (tests call `run_tests` from Rust). B unblocks human use. C is additive. D last so status matches reality. E/F are additive power-feature slices.

---

## 7. Risks & open questions (with verify-by)

| ID | Risk / question | Impact | Verify by |
|---|---|---|---|
| **R1** | `StdContext` has no test-session hook; macros cannot record issues without a core seam | Blocks A | Spike: add defaulted `test_record_issue` on `StdContext` **or** thread a `Cell`/slot via existing host-context — prove one `#expect` fail records without panicking (half-day spike before A) |
| **R2** | `#require` as expression value under `try` / assignment | Wrong unwrap API | Fixture `let x = try #require(… as Int?)`; dump AST (already done) + eval once in spike |
| **R3** | Empty test run exit code (0 vs 1) | CI footgun | Run `swift test` on package with empty test target; lock tswift to same policy |
| **R4** | `.testTarget` + dependency concatenation pulls too much / wrong files | Package tests don’t compile | Fixture Package.swift with `App` + `AppTests` depending on `App`; assert symbols resolve |
| **R5** | Import gate: `import Testing` required vs auto | Ergonomics | Decide in B: auto-import for `tswift test` only; unit-test both with strict gating on |
| **R6** | Async test scheduling edge cases | Flaky fails | Existing concurrency fixtures + one `@Test func t() async` that `await`s |
| **R7** | Pretty-printer drift vs user source (macros, newlines) | Confusing messages | Golden strings for 10 common expr shapes; accept “semantic” not byte-identical |
| **R8** | Suite `init` side effects / missing memberwise init | Can’t construct suite | Document: suites need accessible `init()`; synthesize memberwise only if runtime already does for structs |
| **R9** | Parallelism expectation | Users assume races | Explicit docs: “serial v1”; reopen when multi-interpreter is cheap |
| **R10** | Trait arg parsing (`.tags(.a, .b)`, nested calls) | Skip trait support incomplete | Dump AST for each trait form before implementing; table-driven parser tests |
| **Q1** | Should free tests under a file get a synthetic suite name (filename)? | Output grouping | Prefer Apple-like: free tests ungrouped; suites named |
| **Q2** | `Issue.record` API surface minimum | Slice C scope | Ship `Issue.record(_: String)` only first |
| **Q3** | Zero tests → exit code | See R3 | Checkpoint before B merges |

---

## 8. Non-goals for “done”

This plan is **done** when slices A–D have landed the must-have table and documented later/unsupported items — **not** when swift-testing is fully reimplemented. Reopen for:

- Parallel runner  
- Full throws-matcher / confirmation / known-issue matrix  
- JSON reporter for CI  
- Wasm test entrypoint  

---

## 9. Implementation notes (for the agent who codes)

1. **Read before coding:** `CODING_STANDARD.md`, `docs/agents/environment.md`, `#Predicate` install path in `tswift-swiftdata`, CLI `run()` install order.  
2. **Inspect ASTs** with `tswift dump` / inspect-ast skill — do not guess attribute child shapes.  
3. **Presubmit** before each commit (`scripts/presubmit`, long timeout).  
4. **Commits:** conventional, atomic per slice (`feat(testing): …`, `feat(cli): add tswift test`, …).  
5. **No new crates.io deps** without user OK.  
6. Prefer failing fixture tests that look like real Swift Testing sources over pure Rust mocks of the AST.

---

## 10. Appendix — quick API cheatsheet (user-facing subset)

```swift
import Testing

@Test func free() {
  #expect(2 + 2 == 4)
}

@Test("display name")
func named() throws {
  let n = try #require(Optional.some(1) as Int?)
  #expect(n == 1)
}

@Suite struct Math {
  @Test func ok() { #expect(true) }

  @Test(.disabled("flaky"))
  func skipped() { #expect(false) }

  @Test(arguments: [1, 2, 3])
  func positive(x: Int) { #expect(x > 0) }
}
```

```bash
tswift test Tests/
tswift test . --target AppTests
tswift test Tests/Basic.swift --filter Math
```
