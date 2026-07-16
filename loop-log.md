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

## Coverage iteration — AnyTransition family + .animation(_:)

- Coverage before → after: SwiftUI verified 109 → 118 (15.5% → 16.8%),
  implemented 130 → 131. AnyTransition section 9/2 → 10/10 verified.
- Implemented real `AnyTransition.animation(_:)` (attaches an Animation curve
  to a transition; serializes as a nested `animation` object; nil clears).
- Added `transition` golden fixture verifying the full factory + combinator
  family; new serialization unit test for the curve attachment.
- presubmit green. Blockers: `AnyTransition.modifier(active:identity:)` needs
  arbitrary ViewModifier plumbing — deferred.

## Coverage iteration — Color palette + .opacity/.accentColor

- Coverage before → after: SwiftUI verified 118 → 138 (16.8% → 19.7%),
  implemented 131 → 151. Color section 1/1 → 21/21 verified.
- Implemented `Color.accentColor` and `.opacity(_:)` (real alpha adjust on
  both named tokens and explicit RGB); serializer emits opacity on named
  colors. Credited the system-color palette in scope.toml.
- Added `color-named` golden fixture + named-color-opacity unit test.
- presubmit green. Blockers: `.gradient`/`.mix`/`cgColor`/HDR resolution
  need gradient + color-space plumbing — deferred.

## Coverage iteration — Text typography modifiers

- Coverage before → after: SwiftUI verified 138 → 152 (19.7% → 21.7%),
  implemented 151 → 165. Text section 1/1 → 15/15 verified.
- Implemented kerning/tracking/baselineOffset/monospaced/monospacedDigit and
  registered fontDesign/fontWidth (token-valued, uncredited pending nested
  Font.Design/Font.Width types). Credited the verified Text styling surface.
- Added `text-typography` golden fixture; updated registered-keys expectations.
- presubmit green. Blockers: fontDesign/fontWidth + speech/accessibility text
  members need nested-type token access and a11y plumbing — deferred.

## Coverage iteration — graphic-effect modifiers + Angle

- Coverage before → after: SwiftUI verified 152 → 173 (21.7% → 24.6%),
  implemented 165 → 190. View section 49 → 78 implemented, 70 verified.
- Implemented visual-effect modifiers (blur/brightness/contrast/saturation/
  grayscale/colorInvert/colorMultiply/scaleEffect/rotationEffect/hueRotation)
  and layout/visibility toggles (hidden/allowsHitTesting/lineSpacing/
  minimumScaleFactor/allowsTightening/labelsHidden/help/scrollDisabled).
- Added `Angle` value type (degrees canonical, radians converted) serialized
  as `{"$":"angle","degrees":…}`; new fixture + unit tests.
- presubmit green. Blockers: blendMode/mask/rotation3DEffect need token enums
  or nested-view geometry — deferred.

## Coverage iteration — list & scroll styling modifiers

- Coverage before → after: SwiftUI verified 173 → 186 (24.6% → 26.5%),
  implemented 190 → 203. View section 78 → 96 implemented.
- Implemented compositingGroup/drawingGroup/unredacted, scrollClipDisabled/
  interactiveDismissDisabled/accessibilityHidden/flipsForRightToLeftLayout-
  Direction, listRowSeparator/listSectionSeparator/scrollContentBackground/
  scrollIndicators (Visibility tokens), listRow/SectionSeparatorTint (Color).
- Added `Visibility` token type + `list-scroll-styling` fixture + unit test.
- presubmit green. Blockers: badge/redacted/blendMode/controlSize/imageScale
  need dedicated token enums or nested types — deferred.

## Coverage iteration — token-enum view modifiers

- Coverage before → after: SwiftUI verified 186 → 191 (26.5% → 27.2%),
  implemented 203 → 208.
- Implemented blendMode/controlSize/symbolRenderingMode/redacted/truncationMode
  with dedicated token types (BlendMode/ControlSize/SymbolRenderingMode/
  RedactionReasons/TruncationMode), each serialized as a tagged token.
- Added `token-modifiers` fixture + blendMode unit test.
- presubmit green.

## Coverage iteration — accessibility metadata modifiers

- Coverage before → after: SwiftUI verified 191 → 201 (27.2% → 28.6%),
  implemented 208 → 218. View section 96 → 106 implemented.
- Implemented accessibilityAddTraits/RemoveTraits (AccessibilityTraits token),
  accessibilityHeading (AccessibilityHeadingLevel), accessibilityElement
  (AccessibilityChildBehavior, children:), accessibilitySortPriority (Double),
  accessibilityInputLabels ([String]), and the Bool toggles
  accessibilityIgnoresInvertColors/RespondsToUserInteraction/DirectTouch/
  ShowsLargeContentViewer. Token-valued ones register typed signatures so
  leading-dot members resolve contextually (no cross-namespace collision).
- Added 3 prelude token structs + tag mappings + `accessibility` golden
  fixture + 2 serialization unit tests.
- presubmit green. Blockers: closure/builder-valued a11y modifiers
  (accessibilityAction/Rotor/Representation/Children), accessibilityFocused
  (FocusState), accessibilityTextContentType (`.plain` collides with control
  styles — needs typing the style modifiers) — deferred.

## Coverage iteration — list-editing & identity modifiers

- Coverage before → after: SwiftUI verified 201 → 211 (28.6% → 30.1%),
  implemented 218 → 228. View section 106 → 116 implemented.
- Implemented deleteDisabled/moveDisabled/selectionDisabled (Bool),
  listRowSpacing/listSectionSpacing (CGFloat), badge (Int), id, geometryGroup,
  invalidatableContent (no-arg), interactionActivityTrackingTag (String).
  All record scalar/Bool/String/passthrough values — no leading-dot token, so
  zero cross-namespace collision risk.
- Added `list-editing` golden fixture + serialization unit test.
- presubmit green. Note: `.id(_:)` is recorded as metadata; full view-identity
  semantics (state reset on id change) remain a deeper feature — deferred.

## Coverage iteration — Decimal remainder + Foundation verification

- Coverage before → after: Foundation verified 413 → 443 (67.2% → 72.0%),
  implemented 443 → 445. stdlib 71.2%, SwiftUI 30.1%, SwiftData 8.8% unchanged.
- Implemented real behavior for `Decimal.formTruncatingRemainder(dividingBy:)`
  (`self - other * trunc(self/other)`, sign follows dividend; NaN/zero divisor
  → NaN) and `Decimal.signalingNaN` (mirrors quiet NaN; `isSignaling` stays
  false — Decimal has no distinct signaling NaN). Registry auto-updated.
- Added 3 executable golden fixtures verifying already-implemented members:
  16 URLError.Code cases (badServerResponse…zeroByteResource + `.code`/`failingURL`),
  10 standalone Calendar members (short/veryShort/standalone month/weekday/quarter
  symbols + isDateIn{Today,Tomorrow,Yesterday}), and the new Decimal members
  (formTruncatingRemainder, signalingNaN, isSignaling, `/=`). Unit test for the
  remainder algorithm added to decimal.rs.
- Regenerated website coverage JSON — also corrected pre-existing SwiftUI drift
  (accessibility modifiers registered but JSON stale).
- presubmit green. Blockers: remaining Decimal missing members (`infinity` —
  Decimal cannot represent infinity; `parse`/`parseStrategy`/`consuming` need
  string-index parsing plumbing; `encode`/`hash(into:)` need Codable/Hasher
  seams) deferred.

## Coverage iteration — SwiftUI container-style & text-input modifiers

- Coverage before → after: SwiftUI verified 211 → 224 (30.1% → 31.9%),
  implemented 228 → 244 (32.5% → 34.8%). View section 116 → 132 implemented.
  stdlib 71.2%, Foundation 72.0%, SwiftData 8.8% unchanged.
- Implemented 16 View modifiers. Container/control style setters
  (toggleStyle, menuStyle, gaugeStyle, formStyle, groupBoxStyle,
  labeledContentStyle, indexViewStyle, tabViewStyle, datePickerStyle,
  disclosureGroupStyle, controlGroupStyle) reuse the shared `_ControlStyle`
  token namespace (unique names, host disambiguates by modifier name) —
  extended it with button/borderlessButton/checkbox/columns/page/card/
  navigationLink/accessory*/graphical/compact/field/stepper tokens.
  Text-input modifiers: submitLabel (new SubmitLabel token), textInput-
  Autocapitalization (new TextInputAutocapitalization token), and
  autocorrectionDisabled/disableAutocorrection/focusable Bool toggles.
  New token types wired into `token_of` so they serialize as `{"$":"token"}`.
- Added `style-and-input` golden fixture + 2 serialization unit tests;
  regenerated the hardcoded registered-keys assertion from the registry.
- presubmit green. Blockers: `colorScheme`/`preferredColorScheme` deferred —
  `.light` collides with `FontWeight.light` and needs contextual arg typing;
  same reason `.automatic`-only styles (groupBox/labeledContent/disclosure
  Group) are registered but exercised only via qualified tokens (implemented,
  not verified). imageScale/keyboardType deferred (nested-type / UIKit token).

## Coverage iteration — formIndex + label-aware collection index

- Coverage before → after: stdlib implemented/verified 364 → 368 (71.2% →
  72.0%). Array 65.6%→..., ArraySlice/ContiguousArray/String/Substring each
  gained `formIndex`. Foundation/SwiftUI/SwiftData unchanged.
- Fixed a real gap: `Array.index(after:)`/`index(before:)` previously trapped
  ("index expects two or three integer arguments") because Array's `index` was
  a plain (non-labeled) intrinsic handling only offsetBy forms. Converted
  Array + ArraySlice `index` to label-aware `index_labeled`
  (after:/before:/offsetBy:/limitedBy:), preserving positional-fallback for
  existing callers and base-relative bounds for slices.
- Generalized the dispatcher's `formIndex(after:&i)` inout write-back
  interceptor (previously Set/Dictionary-only) to any builtin receiver with a
  labeled `index` intrinsic, and to all four forms: `formIndex(after:)`,
  `formIndex(before:)`, `formIndex(_:offsetBy:)` (Void), and
  `formIndex(_:offsetBy:limitedBy:)` (returns Bool; writes back only when it
  moved). Registered `formIndex` on Array/ArraySlice/ContiguousArray/String/
  Substring for coverage + fallback.
- Added `stdlib_form_index` golden fixture exercising all forms across Array,
  ArraySlice, ContiguousArray, String, Substring. Updated unit tests to the
  new labeled signature. presubmit green.
- Blockers: Range/ClosedRange/ReversedCollection/CollectionOfOne/
  EmptyCollection still use plain `index` intrinsics — extending formIndex to
  them needs converting those to labeled `index` first (deferred). Remaining
  String/Array missing members are unsafe-pointer/span/Mirror APIs with no
  runtime memory model (infeasible).

## Coverage iteration — URL filesystem-location statics

- Coverage before → after: Foundation URL 37/81 → 55/81 (45.7% → 67.9%);
  Foundation overall 445/615 → 463/615 (72.4% → 75.3%). stdlib/SwiftUI/
  SwiftData unchanged.
- Implemented 16 URL directory statics (temporaryDirectory, homeDirectory,
  documentsDirectory, cachesDirectory, applicationSupportDirectory,
  applicationDirectory, libraryDirectory, desktopDirectory, downloadsDirectory,
  moviesDirectory, musicDirectory, picturesDirectory, sharedPublicDirectory,
  trashDirectory, userDirectory) + currentDirectory() static method, all
  returning file:// directory URLs derived from $HOME / OS temp dir /
  std::env::current_dir. Added dataRepresentation (UTF-8 Data of absolute
  string) and standardizedFileURL (alias of standardized) instance properties.
- Added foundation_url_directories golden fixture; deterministic output uses
  isFileURL/hasDirectoryPath/lastPathComponent so it survives $HOME variance.
  Registered via register_static (BuiltinReceiver::URL). presubmit green.
- Blockers: remaining URL missing members are resource-value/bookmark/security-
  scope/async-bytes APIs (need a real filesystem+metadata model) and
  format/formatted/parse/parseStrategy (need FormatStyle plumbing) — deferred.
  Data's remaining gaps are unsafe-pointer/span/region APIs (no runtime memory
  model, infeasible).

## Coverage iteration — SwiftUI visibility-toggle & speech-hint modifiers

- Coverage before → after: SwiftUI implemented 244 → 262 (34.8% → 37.3%),
  verified 224 → 242 (31.9% → 34.5%). View section +18. stdlib/Foundation/
  SwiftData unchanged.
- Implemented 18 passthrough View modifiers carrying plain Bool/String/Double
  values (no leading-dot token, so no enum-case plumbing needed):
  navigationBarBackButtonHidden, navigationBarHidden, statusBarHidden,
  navigationSubtitle, previewDisplayName, privacySensitive,
  focusEffectDisabled, hoverEffectDisabled, replaceDisabled, findDisabled,
  symbolEffectsRemoved, scrollTargetLayout, scrollIndicatorsFlash,
  allowsWindowActivationEvents, and the accessibility speech hints
  speechAdjustedPitch, speechAlwaysIncludesPunctuation,
  speechAnnouncementsQueued, speechSpellsOutCharacters. All via the shared
  `modifier!` macro + MODIFIER_FNS table; updated the hardcoded
  registered-keys assertion.
- Added visibility-toggles SwiftUI golden fixture verifying serialization of
  all 18. presubmit green.
- Blockers: remaining View modifiers are dominated by token-valued (need enum
  case registration), closure/binding-valued (sheet/popover/onReceive/toolbar),
  and preference/geometry APIs — each needs more than a passthrough record.

## Coverage iteration — SwiftUI visibility-token & scalar layout modifiers

- Coverage before → after: SwiftUI implemented 262 → 272 (37.3% → 38.7%),
  verified 242 → 252 (34.5% → 35.9%). View section +10. stdlib/Foundation/
  SwiftData unchanged.
- Implemented 10 passthrough View modifiers: Visibility-token chrome modifiers
  (persistentSystemOverlays, menuIndicator, listSectionIndexVisibility,
  navigationLinkIndicatorVisibility — exercised with `.visible`/`.hidden`;
  `.automatic` still collides across enums) and scalar layout modifiers
  (gridCellColumns Int span; labelIconToTitleSpacing, labelReservedIconWidth,
  inspectorColumnWidth, navigationSplitViewColumnWidth,
  defaultWheelPickerItemHeight CGFloat). All via `modifier!` + MODIFIER_FNS;
  updated the hardcoded registered-keys assertion.
- Added chrome-and-layout SwiftUI golden fixture. presubmit green.
- Blockers: `.automatic`-token modifiers need contextual enum-typing;
  remaining View modifiers are closure/binding/preference/geometry APIs.

## Coverage iteration — SwiftUI label/progress/table/navigation style setters

- Coverage before → after: SwiftUI implemented 272 → 278 (38.7% → 39.6%),
  verified 252 → 258 (35.9% → 36.8%). View section +6. Other frameworks
  unchanged.
- Implemented 6 style-setter View modifiers reusing the shared `_ControlStyle`
  token namespace: labelStyle (.iconOnly/.titleOnly/.titleAndIcon),
  progressViewStyle (.circular), textEditorStyle (.plain), tableStyle (.inset),
  navigationViewStyle (.stack/.columns), navigationSplitViewStyle
  (.balanced/.prominentDetail). Added 7 unique tokens to `_ControlStyle`
  (iconOnly, titleOnly, titleAndIcon, circular, stack, balanced,
  prominentDetail). `.linear` deliberately not added for progressViewStyle —
  collides with Animation.linear in the single leading-dot namespace.
- Added more-styles SwiftUI golden fixture. presubmit green.
- Blockers: `.automatic`/`.linear` and other cross-enum-colliding tokens still
  need contextual enum typing; remaining View modifiers are
  closure/binding/preference/geometry APIs.

## Coverage iteration — SwiftUI prominence & button-border-shape modifiers

- Coverage before → after: SwiftUI implemented 278 → 281 (39.6% → 40.0%),
  verified 258 → 261 (36.8% → 37.2%). View section +3. Others unchanged.
- Implemented headerProminence/badgeProminence (.increased/.standard/
  .decreased) and buttonBorderShape (.roundedRectangle/.capsule/.circle),
  reusing the shared `_ControlStyle` token namespace; added 6 new unique
  tokens. Added prominence-shapes SwiftUI golden fixture. presubmit green.
- Blockers: same as prior SwiftUI iterations — remaining modifiers are
  cross-enum-colliding tokens (need contextual typing) or closure/binding/
  preference/geometry APIs.

## Coverage iteration — Double sign/width & masking-shift assignments

- Coverage before → after: stdlib implemented 368 → 374 (72.0% → 73.2%),
  verified 368 → 374 (fully verified). Foundation/SwiftUI/SwiftData unchanged.
- Pivoted off the diminishing-returns SwiftUI colliding-token modifiers to
  real-behavior stdlib numerics. Implemented Double.sign (returns a
  FloatingPointSign enum via is_sign_negative), Double.significandWidth
  (MSB−LSB span of the significand magnitude, −1 for zero/non-finite), and
  the Double.quietNaN/signalingNaN type constants (double_type_constant).
  Added &<<= / &>>= masking-shift compound assignments to the parser's
  is_assignment set so they fold through ops.rs's existing &<< / &>> arms.
- Registered new keys, updated stdlib scope.toml core_members (Int +
  &<<=/&>>=, Double + quietNaN/signalingNaN) and the coverage tool's
  _OP_TOKENS (added &<<=/&>>=). Two new golden fixtures. presubmit green.
- Blockers: remaining stdlib misses are dominated by unsafe-pointer/span
  APIs, customMirror/hash/encode reflection hooks, and init/subscript that
  need dedicated dispatch — each needs more than a passthrough.

## Coverage iteration — SwiftUI value & nested-view modifiers

- Coverage before → after: SwiftUI implemented 281 → 290 (40.0% → 41.3%),
  verified 261 → 270 (37.2% → 38.5%). View section +9. Other frameworks
  unchanged.
- Broke the colliding-token plateau by targeting modifiers whose args are
  NOT leading-dot tokens. Value passthroughs via the shared `modifier!`
  macro: position (x:y: CGFloat), accentColor (Color), safeAreaPadding
  (CGFloat), listRowInsets (EdgeInsets), navigationBarTitle (String),
  lineHeight (CGFloat). Nested-view records reusing the overlay/background
  `compose_modifier` route: mask, contextMenu, listRowBackground.
- Added an EdgeInsets builtin struct to the SwiftUI prelude (top/leading/
  bottom/trailing, plus zero init) with tagged UIIR serialization
  (`{"$":"edgeInsets",…}`) so listRowInsets carries a real value.
- Updated MODIFIER_FNS, the hardcoded registered-keys assertion, and added
  a value-and-nested-modifiers golden fixture. presubmit green.
- Blockers: remaining View modifiers still dominated by leading-dot token
  args (need contextual enum typing to disambiguate the shared namespace),
  preference/geometry/anchor APIs, and closure-heavy presentation modifiers
  (sheet/popover/alert) that need binding + dismissal plumbing.

## Coverage iteration — SwiftUI single-closure event handlers

- Coverage before → after: SwiftUI implemented 290 → 297 (41.3% → 42.3%),
  verified 270 → 277 (38.5% → 39.5%). View section +7. Others unchanged.
- Added an `event_handler!` macro (records a marker + binds the trailing
  closure under a distinct handler key, ADR-0013 §3) and used it for seven
  non-colliding event modifiers: onHover, onOpenURL, refreshable,
  onDeleteCommand, onExitCommand, onPlayPauseCommand, onDrag. Closures never
  serialize — only the marker reaches the UIIR — so hosts wire the listener.
- Updated MODIFIER_FNS, the registered-keys assertion, and added an
  event-handler-modifiers golden fixture. presubmit green.
- Blockers: remaining are token-arg modifiers (contextual enum typing),
  binding-driven presentation (sheet/popover/alert/fullScreenCover need
  isPresented binding + dismissal), and preference/anchor/geometry APIs.

## Coverage iteration — SwiftUI edit/pencil/hover command handlers

- Coverage before → after: SwiftUI implemented 297 → 303 (42.3% → 43.2%),
  verified 277 → 283 (39.5% → 40.3%, crossing 40% verified). View +6.
- Reused the event_handler! macro for six more non-colliding single-closure
  modifiers: onCutCommand, onCopyCommand, onMoveCommand, onPencilDoubleTap,
  onPencilSqueeze, onContinuousHover. Updated MODIFIER_FNS, the registered-
  keys assertion, and added a command-handler-modifiers golden fixture.
  presubmit green.
- Blockers: unchanged — token-arg modifiers, binding-driven presentation,
  and preference/anchor/geometry APIs remain the hard remainder.

## Coverage iteration — SwiftUI marker & value passthroughs

- Coverage before → after: SwiftUI implemented 303 → 308 (43.2% → 43.9%),
  verified 283 → 288 (40.3% → 41.0%). View +5. Others unchanged.
- Added five non-colliding modifiers: no-arg marker overloads whose args
  are all defaulted (equatable, focusSection, ignoresSafeArea) plus
  single-value passthroughs (coordinateSpace(name:), draggable). All via
  `modifier!` + MODIFIER_FNS; updated the registered-keys assertion; added
  a marker-and-value-modifiers golden fixture. presubmit green.
- Confirmed the token-modifier blocker is real: leading-dot resolution is
  by global uniqueness across one namespace, so e.g. imageScale's
  .small/.medium/.large collide with ControlSize/FontWeight, and
  colorScheme's .light collides with FontWeight.light. Breaking this needs
  contextual enum typing (modifier-parameter-driven token resolution).
