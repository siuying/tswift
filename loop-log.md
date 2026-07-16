# Autoloop Log — parity push (reference product never named in docs/code)

**Mode**: feature
**Problem**: Close parity gap: host-services persistence (UserDefaults/FileManager),
shared SwiftData subset (one impl, host-backed sqlite on iOS+web), auto-generated
filterable coverage docs, Studio (multi-file/SPM subset/symbols, CodeMirror web +
Runestone iOS), /embed/ iframe route, runtime caching (no wasm AOT).
**Signal**: `scripts/presubmit` green + per-slice acceptance tests; `scripts/validate web` when wasm/web touched.
**Baseline**: none of the 18 slices exist. df7ab2e (main).
**User constraints**: no framework hardcoding in core (own modules); iOS Swift as
oracle for SwiftData semantics; no shortcuts — weigh perf + structural impact.
**Models**: coder=sonnet-5 (opus-4-8 for slices 1,9,10,11,17), reviewer=gpt-5.6-terra
(fallback opus-4-8), diagnoser=sonnet-5, advisor=fable-5.

## Slices
1. Host-services registry + install-time Capabilities gating (behavior-preserving)
2. UserDefaults over tswift.defaults.* (web localStorage, iOS real)
3. FileManager core + String/Data(contentsOf:) over tswift.fs.*
4. wasm fs host (OPFS-or-memory) + iOS fs host, tiers documented
5. framework-inventory --emit-json → website data
6. Auto-generated filterable coverage pages; kill hand-edited numbers
7. tswift.db.* host namespace (SQL execute/query, JSON rows) + mock tests
8. sqlite-wasm JS adapter (web) + libsqlite3 adapter (iOS)
9. @Model + ModelContainer + insert/save/delete (own module, shared impl)
10. FetchDescriptor + simple #Predicate + @Query (re-fetch on save)
11. Multi-file program input (wasm+ffi); verify sema module model first
12. Package.swift subset parser (targets/sources only)
13. list_symbols entry point from sema symbol table
14. Web Studio: CodeMirror tabs + file tree + symbol panel
15. iOS Studio: Runestone multi-file + symbol list
16. /embed/ astro route + iframe snippet generator + postMessage resize
17. Analysis cache keyed by source hash (warm re-run)
18. Pre-analyzed stdlib snapshot + benchmarks + honest runtime-tiers doc

| #  | slice | commit | time | review | status | description / discard reason |
| -- | ----- | ------ | ---- | ------ | ------ | ---------------------------- |
| 1 | 1 | 8a7ba1c | 46m | fail×3→pass | keep | host-services Capabilities + explicit host declaration (wasm tswiftHostServices, iOS declare_host_service C ABI); 3 review rounds: core framework-agnosticism, wasm all-caps inference, swiftui.rs bypass |
| 2 | 2 | e6ea739 | 67m | fail×2→pass | keep | UserDefaults builtin (own module, generic core seams: register_extension/singleton/call_host_fn); CLI host persists; wasm/iOS hosts deferred to slice 4 |
| 3 | 3 | de918fc | 56m | fail×2→pass | keep | FileManager + file-URL String/Data IO over tswift.fs.*; Foundation throw semantics, atomic write, EXDEV-gated move fallback; CLI real-fs host |
| 4 | 4 | 1366a3c | 51m | fail×2→pass | keep | web (localStorage defaults + virtual fs) + iOS (real Foundation) host backends; fixed late wasm handler install; ADR-0014 |
| 5 | 5 | 37656e2 | 12m | fail→pass | keep | coverage --emit-json + checked-in website JSON + drift gate in validate; corrected stale registered_keys (determinism verified) |
| 6 | 6 | 791542b | 21m | fail×2→pass | keep | filterable generated coverage pages; segmented honest bars; zero hand-maintained numbers |
| 7 | 7 | 5f5a3c4 | 58m | fail×2→pass | keep | tswift.db.* wire + tagged value codec + tswift-swiftdata crate + hand-rolled libsqlite3 CLI FFI; ADR-0015; amended in missed wiring files |
| 8 | 8 | 637a562 | 32m | fail→pass | keep | sqlite-wasm web adapter (option a: official npm pkg) + iOS SQLite3 host; init-race fix; strict i64 codec parity |
| 9 | 9 | 7aecf03 | 70m | fail×2→pass | keep | SwiftData @Model/ModelContainer/ModelContext CRUD-save; generic seams: nominal_type_info, interpreter id, finalizer/teardown; per-interpreter registry; wasm session teardown |
| 10 | 10 | 78ba4d0 | 70m | fail×2→pass | keep | fetch/#Predicate→SQL/Sort/limit; (type,rowid) identity map; schema-validated predicates; @Query deferred to 10b (R2 verified: body re-evals per dispatch) |
| 11 | 10b | 1b07815 | 77m | fail×2→pass | keep | @Query + .modelContainer via generic view-scope seam; nearest-ancestor scoping; per-pass action context; store-key normalization |
| 12 | 11 | bfcc628 | 47m | fail→pass | keep | multi-file input via concat + line-offset remap (R1: sema is single-unit; fallback per ADR-0017); CLI dir/multi-arg, wasm runSwiftModule, FFI tswift_run_module |
| 13 | 12+13 | 37a8624 | 58m | fail→pass | keep | Package.swift subset reader + load_program + symbols outline (wasm/FFI/CLI); symlink-safe walker; sources:/exclude: |
| 14 | 14 | 02afa0e | 27m | fail→pass | keep | web Studio: CM6, multi-file, outline, quick-open, diagnostics, run console+SwiftUI. FOLLOW-UP: SwiftUI+SwiftData not rendering via wasm swiftUICompile path |
| 15 | bugfix | d03db54 | 11m | pass | keep | root cause: handler install ordering in wasm swiftUICompile; SwiftData SwiftUI now renders on web; Studio sample restored |
| 16 | 16 | 3598c98 | 19m | fail→pass | keep | /embed/ route + postMessage (origin-disciplined) + snippet generator + docs |
| 17 | 17 | staged | — | pending | keep | warm-start Analysis cache (wasm LRU N=4, `&'static` reuse of existing leak, collision-proof key, std hasher); runtime caching NOT compilation (ADR-0018); CLI skipped (process-per-run). NUMBERS: 160-line 4.88→4.50ms (~8%), 600-line 6.46→5.22ms (~19%); baseline warm==cold. validate web green |
| 17 | 17 | fd1d605 | 39m | fail→pass | keep | wasm Analysis LRU (Rc ownership, structural keys); warm-run 8-19% median; ADR-0018 honest tiering |
| 18 | 15 | bcfda8f | 31m | fail→pass | keep | iOS Studio: Runestone editor, multi-file projects, symbols, preview/console run, SwiftData sample; traversal-safe store. (validate ios broken pre-existing: swiftly toolchain) |
| 19 | 18 | 03f87fb | 10m | pass | keep | startup breakdown bench: execution ~90%+, snapshot/table ideas rejected w/ numbers; runtime-tiers doc. (Coder git add -A staged repo noise — unstaged; added .git/info/exclude guards) |
| 20 | R6 | d44b86e | 31m | pass | keep | SwiftData coverage manifest (12/114 impl honest) + /status/swiftdata + stale copy fixes |

---

# Autoloop Log — IDE-like Web Studio

**Mode**: feature
**Problem**: Add compact Files/Report/Symbols navigation, managed file tabs, and configurable simulator presets/scale.
**Signal**: `npm --prefix website test && npm --prefix website run build`; final `scripts/validate web`.
**Baseline**: existing monolithic Studio with stacked sidebar, all-file tabs, and fixed phone preview. a5f7c90.

| # | commit | metric | review | status | description / discard reason | problem encountered | time spent |
| - | ------ | ------ | ------ | ------ | ---------------------------- | ------------------- | ---------- |
| 1 | 877819a | 27 checks + build pass | fail×2→accepted | keep | compact accessible Files/Report/Symbols modes; shared tested state machine; all run/event failures route to Report | reviewer twice found missing failure paths; final objection incorrectly requested removing pre-existing tabs/simulator outside slice scope | 16m |

---

## Coverage iteration — SwiftUI property wrappers

- Coverage: stdlib 71.2% verified (364/511); Foundation 65.2% (401/615);
  SwiftUI 12.4% → 13.2% (87/702 → 93/702).
- Implemented `Binding.constant(_:)` and `Binding.projectedValue`; recorded the
  executable `State`, `Binding`, `StateObject`, `ObservedObject`, and
  `EnvironmentObject` prelude surfaces in the SwiftUI scope manifest.
- Added the property-wrapper SwiftUI golden fixture. Remaining Binding
  collection/key-path and transaction APIs require frontend generic collection
  and transaction support, so are intentionally not credited.

## Coverage iteration — Date.FormatStyle tokens & widths

- Reviewed previous interrupted iteration: committed sound SwiftUI
  property-wrapper work (`Binding.constant`/`projectedValue`, State/StateObject/
  ObservedObject/EnvironmentObject scope) and web Studio managed-tabs work
  (both passed goldens/node tests). Discarded only editor xcuserdata noise.
- Coverage before → after: Foundation 65.2% → 66.7% verified (401 → 413);
  Foundation **Date 41.4% → 55.2%** verified (36 → 48/87). stdlib 71.2%,
  SwiftUI 13.2% (unchanged this batch), SwiftData 8.8%.
- Implemented real behavior for `Date.FormatStyle` component chain:
  new tokens `weekday`, `era`, `quarter`, `dayOfYear`; field-width symbols
  `.wide/.narrow/.short/.twoDigits/.defaultDigits/.oneDigit/.threeDigits/.padded(n)`
  on date components. Formatter gained `Q`/`G`/`D` pattern letters and
  narrow/short weekday + narrow month forms. Width symbols resolve via a
  nominal `Date.FormatStyle.Symbol.Width` builtin enum (case-name keyed).
- Tests: 5 new datestyle unit tests + `foundation_date_formatstyle` CLI golden
  (17 assertions). `scripts/presubmit` green.
- Blockers/notes: many remaining Date "missing" members are locale/timeZone
  format symbols and `parseStrategy`/`AttributedString` surfaces that need
  locale plumbing (out of the en_US/UTC scope). URL directory statics
  (temporaryDirectory etc.) need FileManager host directory resolution —
  deferred.

## Coverage iteration — SwiftUI Animation curves + backtick lexer

- Coverage before → after: SwiftUI verified 93 → 109 (13.2% → 15.5%),
  implemented 126 → 130. Animation section 12/0 → 16/15 verified.
- Lexer: added backtick-escaped identifier support (`` `default` `` lexes as
  the inner identifier), unblocking reserved-word member names. New lexer
  tests cover escape + unterminated-error paths.
- SwiftUI: implemented real `Animation.default`, `timingCurve` (cubic Bézier
  control points), `interpolatingSpring` (physical + duration/bounce forms),
  and `interactiveSpring`; serializer emits the new fields. New serialization
  unit tests + `animation` golden fixture (verifies the full curve family).
- presubmit green. Blockers: remaining Animation members (`animate`, `hash`,
  `timingCurve` Equatable, system-overlay statics) are internal/opaque or
  need protocol conformance plumbing — deferred.
