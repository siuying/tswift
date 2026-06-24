---
date: 2026-06-24T21:20:40Z
git_commit: 0f92a4a240cb1f8e8476bfc2c1c037d4d40bf682
branch: main
repository: toprakdeviren/msf (checked out under .checkout/msf)
topic: "How msf works as a Swift frontend, its relationship to miniswift.run, and how to build on it for compiler/editor work"
tags: [research, codebase, msf, swift, compiler-frontend, lexer, parser, sema, wasm, swiftwasm]
status: complete
last_updated: 2026-06-24
---

# Research: msf (Mini Swift Frontend) — architecture, capabilities, and how to build on it

**Date**: 2026-06-24T21:20:40Z
**Subject repo**: https://github.com/toprakdeviren/msf @ `0f92a4a` (main)
**Checked out at**: `.checkout/msf`

## Research Question

Study the `msf` repository to understand:
1. How it works (and how it does *not* "put all of Swift in one C file").
2. How the related demo at https://miniswift.run/ "live-compiles and runs SwiftUI in the browser."
3. How we could use the **public** msf library ourselves — without miniswift's private backend — to build an online Swift editor and/or compile-and-run Swift apps, including on the web and embedded in an iOS app.

This document records the current state for **future compiler work**.

## Summary

`msf` is the **front third of a Swift compiler**, shipped as a C11 static library
(`libMiniSwiftFrontend.a`) with a single public header (`include/msf.h`). It does
**lexing → parsing → semantic analysis** and produces a fully typed, immutable AST.
It has **no IR, no code generation, no optimizer, no runtime, and no SwiftUI** — by
design. The README states it outright: *"No LLVM, no codegen, no runtime — just the
frontend."*

The "single-header" claim means a **single public include surface** (`msf.h`), not a
single translation unit. The implementation is ~50 `.c` files / ~48K LOC. The
`generated/` directory (~254K lines) is mostly **data tables** — the largest being
`sdk_vocab.h` (187K lines), a serialized snapshot of the Apple SDK's public type
surface used for SDK-free type resolution.

**miniswift.run** is `msf` (the open frontend) **plus a proprietary middle/back end**
(IR gen → SSA → optimizer → direct-to-WASM codegen), a **Swift runtime/stdlib/Foundation**,
a **from-scratch SwiftUI reimplementation** (their "UIIR" + canvas renderer + identity-aware
diff engine), and a **Metal→WGSL** compiler. The site advertises 71K lines of C
("compiler · stdlib · foundation · swiftui · metal"); the public repo is only the
frontend, so the parts that actually *run* a SwiftUI app are **not in the repo**.

For our own work: **msf can fully power a smart Swift editor** (diagnostics,
highlighting, completion, hover types, AST view) on web (WASM) or natively inside an
iOS app (link the C lib). It **cannot run code**; running requires a backend + runtime
we would have to build or source elsewhere (e.g. SwiftWasm with a compile server).

## Detailed Findings

### 1. Pipeline & structure

Entry point `analyze_impl()` runs three stages in sequence (`src/msf.c:94-126`):

```c
lexer_tokenize(&r->src, &r->ts, 1, &r->lex_diag);  // 1. Tokenize
r->root = parse_source_file(r->parser);            // 2. Parse
sema_analyze(r->sema, r->root);                    // 3. Sema
```

Module layout (from README "Project structure" + `Makefile` source globs):
- `src/msf.c` — pipeline entry + whole-module (`MSFModule`) driver
- `src/lexer/` — tokenizer (+ `scan/` fast paths)
- `src/parser/` — recursive descent + Pratt expression parsing
- `src/type/` — type arena, builtin singletons, equality, substitution
- `src/semantic/` — 3-pass sema (declare/resolve/conform)
- `src/unicode/` — vendored NFC normalization
- `src/vocab.c` — `.swiftinterface` → portable `.msfvocab`
- `src/project.c` — Xcode/SwiftPM project discovery (POSIX-only)
- `generated/` — committed codegen: AST/type kind tables, keyword map, baked SDK vocab

The build (`Makefile`) compiles all `.c` into `libMiniSwiftFrontend.a`; consumers
`#include <msf.h>` only. `make wasm` produces a WASM `.a` (no JS glue / no Emscripten
exports are defined by the repo — see §4).

### 2. Lexer SWAR fast-path (`src/lexer/scan/fast.c`)

Optimized for the common case (~95% identifiers/keywords):
- **`scan_ident`** — ASCII fast path reads 8 bytes at a time into a `uint64_t`,
  rejects non-ASCII with `word & 0x8080808080808080ULL`, and validates all 8 lanes
  against a **256-bit identifier bitmap** (`LEX_IDENT_BITMAP[2]`). Tail loop handles
  remaining bytes + multi-byte UTF-8 via `unicode_is_ident_head/continue`.
- **`keyword_detect`** — binary search over the 67-entry sorted `LEX_KEYWORDS[]`,
  length-gated (2–16) with first-byte fast reject then bounded `memcmp`.
- **`scan_string_body`** — dual `memchr` (closing quote + backslash), jumps to
  whichever is first; `memchr` lowers to NEON/AVX2/WASM128 SIMD. `skip_str_interp`
  handles `\( … )` interpolation by balancing parens AND skipping nested strings,
  bounded by `guard=64`.
- **`scan_number`** — decimal/hex/octal/binary, underscores, fractions, exponents.

### 3. Three-pass semantic analyzer (`src/semantic/resolve/resolver.c`)

Key design: **single-file analysis is whole-module analysis with one file** —
`sema_analyze()` wraps the root in a one-element array and calls
`sema_analyze_module()` (`resolver.c:797-811`). Passes run **breadth-first across all
files** (`resolver.c:879-901`):
1. **Pass 1 — Declare** (`semantic/declare.c:624`, `declare_node`): forward-register
   every symbol so forward/cross-file references resolve.
2. **Pass 2 — Resolve** (`resolve_node`): bottom-up type inference; each node gets a
   `TypeInfo*`. Expressions fan out via the case dispatcher
   (`semantic/resolve/expression/dispatch.c`) → `binary.c`, `call.c` (overloads),
   `member.c`.
3. **Witness index** (between 2 and 3): gather members across body + extensions +
   inheritance graph so conformance can be satisfied by a witness in any file.
4. **Pass 3 — Conform** (`pass3_check_conformances`) + `Sendable` inference/closure
   checks.

All files share one `SemaContext` (symbol table, type arena, intern pool,
conformance tables). `sema_switch_file()` (`resolver.c:823`) swaps only
`src`/`tokens`/`root` and clears the per-file `tok_cache`. No text concatenation, no
SDK stubs — cross-file refs resolve purely by interned name.

### 4. Editor-relevant public API (`include/msf.h`)

Everything an editor needs is present:
- Diagnostics: `msf_error_count/_message/_line/_col` and byte ranges
  `msf_error_start_offset`/`msf_error_end_offset` (`msf.h:1281-1326`).
- Tokens: `msf_tokens`, `msf_token_count`, `token_type_name`, `token_text`.
- AST: `msf_root`, `ast_kind_name`, walk `first_child`/`next_sibling`; `ASTNode.type`
  is a resolved `TypeInfo*`; `type_to_string`.
- Serialization: `msf_dump_json` / `_text` / `_sexpr` (`msf.h:1341-1355`).
- Completion data: `msf_vocab_find_member` (`msf.h:1015`), `msf_vocab_module_types`
  (`msf.h:1051`).
- Multi-file: `MSFModule` (`msf_module_*`); imports via `msf_analyze_with_vocab` /
  `msf_module_set_vocabulary`.

Sections 9–16 ("Backend ABI") expose the runtime *shapes* (`ASTNode.modifiers`,
`TypeArena`, `type_substitute`, `ConformanceTable`, assoc-type bindings) for a backend
that lowers the AST — i.e. the seam where miniswift's private backend attaches.

### 5. miniswift.run — what the demo actually is

Confirmed from the site's own copy and assets:
- Two lowering paths:
  - General Swift: `.swift → Lexer → Parser → Sema → IR Gen → SSA → Optimizer → WASM`
    (direct-to-WASM; "No LLVM, no clang, no Binaryen").
  - SwiftUI: `swift → lexer → parser → sema → UIIR → canvas` — `body` evaluated to a
    JSON "UIIR" tree, painted to `<canvas>` by a JS renderer; `@State` mutation →
    identity-aware diff → repaint only the changed subtree.
- Browser assets: `wasm/miniswift.js` (+ `.wasm`, the compiler+runtime),
  `swiftui.min.js` (SwiftUI runtime/renderer/diff), `js/metal/*` (MSL→MIR→WGSL→WebGPU),
  `vs/loader.js` (Monaco).
- Claims: 71K lines of C, 52 SwiftUI views, 75 modifiers, 290 runtime functions,
  751 tests, 0 npm/LLVM/clang deps. All client-side, no server.

The `msf` repo is the open **frontend** of this stack; the IR/optimizer/WASM
backend + Swift runtime + SwiftUI reimplementation + renderer are **closed** and not
in the repo (~the 23K-LOC difference + the JS).

### 6. Using msf ourselves (without miniswift's private backend)

**Can build today with public msf:** a smart Swift editor.

| Feature | API |
|---|---|
| Diagnostics/squiggles | `msf_error_*` + start/end offsets |
| Syntax highlight | `msf_tokens` + `token_type_name`/`token_text` |
| Outline | `msf_root` + `ast_kind_name` |
| Hover types | `ASTNode.type` + `type_to_string` |
| AST view | `msf_dump_json` |
| Completion | `msf_vocab_find_member`, `msf_vocab_module_types` |
| Multi-file | `MSFModule` |

License is **MIT** → commercial use OK.

**WASM wiring:** `make wasm` yields only the `.a`. We must write a C shim and link it
with Emscripten, defining our own exports, e.g.:

```bash
emcc shim.c build/wasm/libMiniSwiftFrontend.a -Iinclude -Igenerated -O2 -msimd128 \
  -s EXPORTED_FUNCTIONS='["_analyze_to_json","_free","_malloc"]' \
  -s EXPORTED_RUNTIME_METHODS='["ccall","cwrap","UTF8ToString","stringToUTF8"]' \
  -s MODULARIZE=1 -o msf.js
```
Shim calls `msf_analyze`, serializes errors+tokens+AST (`open_memstream` +
`msf_dump_json`) to a JSON string for JS. Pair with Monaco/CodeMirror.

**Cannot do with msf alone:** run code. No IR/codegen/runtime/SwiftUI in repo.

### 7. Running Swift without the private backend — options

- **A. Build our own backend on msf** (Backend ABI §9–16): AST→IR→WASM (or
  interpreter) + Swift runtime + (for UI) a SwiftUI reimplementation. This is the
  ~person-years piece miniswift kept private. msf gives only the frontend third.
- **B. SwiftWasm** (https://swiftwasm.org): real `swiftc` (LLVM) targeting
  `wasm32-unknown-wasi`. **The compiler does NOT run in the browser/on-device** — it
  runs on a build machine. JavaScriptKit bridges DOM/JS; Tokamak is a SwiftUI-like
  DOM renderer but largely unmaintained — do not bet a product on it. **SwiftUI proper
  does not exist on SwiftWasm.**
- **C. Hybrid (recommended):** msf for instant client-side intelligence; a server
  running SwiftWasm/`swiftc` for actual compile+run.

### 8. Embedding constraints (web vs iOS)

**Web:**
- Prebuilt SwiftWasm app → easy to embed (ship `.wasm` + loader; `<iframe>`/div).
- Live-compile of arbitrary user Swift → **needs a compile server** (swiftc too large
  to run in-browser). Flow: editor → POST source → server `swiftc → .wasm` → return →
  browser runs `.wasm`. This is precisely why miniswift wrote a small C compiler that
  *can* run client-side.

**iOS app:**
- Editing UI: trivial (Runestone / `UITextView` / Monaco-in-`WKWebView`).
- On-device compilation: **not possible** (no Swift toolchain on iOS, no JIT for own
  process). Compile on server, same as web.
- Running compiled output: native execution of downloaded code is barred by App Store
  rule **2.5.2**. Sanctioned path: run the compiled **`.wasm` inside `WKWebView`**
  (the only iOS context granted JIT) or a pure-interpreter wasm runtime in-process.
  Apple's Swift Playgrounds uses a private interpreter + entitlements unavailable to
  third parties.
- **msf fits iOS especially well**: it's a C static lib — link `libMiniSwiftFrontend.a`
  into the app target and call `msf_analyze` natively for offline diagnostics/completion;
  no WASM/WebView needed for the editor-intelligence layer.

| Capability | Web | iOS app |
|---|---|---|
| Editor UI | Monaco/CodeMirror | Runestone / WKWebView |
| Live diagnostics/completion/types | msf → WASM | msf native C lib |
| Compile arbitrary Swift | server (SwiftWasm) | server (SwiftWasm) |
| Run compiled output | `.wasm` in tab | `.wasm` in `WKWebView` |
| Run SwiftUI | reimpl needed (Tokamak dying) | reimpl needed |

## Code References

- `.checkout/msf/src/msf.c:94-126` — three-stage pipeline (tokenize/parse/sema)
- `.checkout/msf/src/msf.c:131-141` — `msf_analyze` / `_in_module` / `_with_vocab`
- `.checkout/msf/src/lexer/scan/fast.c:33` — `keyword_detect` (binary search)
- `.checkout/msf/src/lexer/scan/fast.c:75-94` — `LEX_IDENT_BITMAP` + `IDENT_OK`
- `.checkout/msf/src/lexer/scan/fast.c:100-160` — `scan_ident` SWAR fast-path
- `.checkout/msf/src/lexer/scan/fast.c` (string section) — `scan_string_body` dual memchr + `skip_str_interp`
- `.checkout/msf/src/semantic/resolve/resolver.c:797-811` — `sema_analyze` (1-file = module)
- `.checkout/msf/src/semantic/resolve/resolver.c:823-839` — `sema_switch_file`
- `.checkout/msf/src/semantic/resolve/resolver.c:879-901` — pass 1/2/witness/pass 3 loops
- `.checkout/msf/src/semantic/declare.c:624` — `declare_node` (forward declaration)
- `.checkout/msf/include/msf.h:1281-1326` — error/offset API
- `.checkout/msf/include/msf.h:1341-1355` — dump API
- `.checkout/msf/include/msf.h:1015,1051` — vocab member/type lookup
- `.checkout/msf/include/msf.h:1362-1370` — Backend ABI note (backend seam)
- `.checkout/msf/Makefile` — source globs, `wasm`/`release`/`sdk-vocab` targets

## Architecture Documentation (patterns observed)

- **Single public header**, many implementation TUs compiled to one static lib.
- **Arena allocation** for AST + `TypeInfo`; bulk-free via `msf_result_free`.
- **Zero-copy tokens** (offset+length into source); source must outlive result.
- **Pointer-identity builtins** (`TY_BUILTIN_INT` etc.); type checks via `==`.
- **String interning** (FNV-1a + NFC); symbol lookup is pointer equality.
- **Table-driven dispatch** (char class, AST-kind function tables, keyword map).
- **SDK-free resolution** via portable baked vocabulary (`sdk_vocab*.h` / `.msfvocab`),
  enabling browser/Windows analysis with no toolchain.
- **Frontend/backend seam** is the documented "Backend ABI" (msf.h §9–16).

## Open Questions / Future Work

- If we pursue our own backend (Option A): design the AST→IR lowering against the
  Backend ABI; decide interpreter vs direct-WASM; scope a minimal runtime/stdlib.
- If Option C (hybrid): design the compile-server (sandboxing, caching, timeouts,
  abuse limits) and the SwiftWasm SDK invocation; define the editor↔server protocol.
- iOS: confirm current App Store review treatment of `WKWebView`-hosted wasm execution
  for a code-runner app; evaluate in-process wasm interpreters (perf vs policy).
- Evaluate whether `msf`'s vocabulary coverage (`sdk_vocab_web.h`) is sufficient for
  our target SDK surface, or whether we need to regenerate via `make sdk-vocab`.
- SwiftUI-in-browser without miniswift: assess effort to build a minimal SwiftUI-like
  renderer (or revive/fork Tokamak) vs. scope of supported views/modifiers.
