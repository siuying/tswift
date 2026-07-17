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

## Coverage iteration — SwiftUI token modifiers via typed seam

- Coverage before → after: SwiftUI implemented 308 → 317 (43.9% → 45.2%),
  verified 288 → 297 (41.0% → 42.3%). View +9. Others unchanged.
- BROKE THE TOKEN PLATEAU. Discovered register_struct_method_typed: it
  pushes a contextual parameter type so a leading-dot arg resolves against
  THAT type instead of by global uniqueness (the collision blocker). Added
  nine token modifiers each with a dedicated namespace: colorScheme +
  preferredColorScheme (ColorScheme .light/.dark), symbolVariant
  (SymbolVariants .fill/.circle/…), hoverEffect, menuOrder,
  contentTransition, scrollBounceBehavior, scrollDismissesKeyboard,
  dynamicTypeSize. Added the 8 token structs to the PRELUDE, token_of, and
  the UIIR tag map.
- Cascade fix: the new types reuse shared names (.small/.medium/.large/
  .light/.circle), so the previously uniqueness-resolved controlSize,
  fontWeight, and buttonBorderShape were converted to the typed seam to keep
  their tokens resolving. presubmit green.
- Unlocks a path for the remaining token modifiers (imageScale,
  keyboardType, menuActionDismissBehavior, etc.) — each just needs a typed
  namespace + typed registration.

## Coverage iteration — SwiftUI input/image/behavior token modifiers

- Coverage before → after: SwiftUI implemented 317 → 326 (45.2% → 46.4%),
  verified 297 → 306 (42.3% → 43.6%). View +9. Others unchanged.
- Second typed-seam batch: imageScale, keyboardType, autocapitalization,
  menuActionDismissBehavior, buttonRepeatBehavior, textScale,
  writingToolsBehavior, allowedDynamicRange, labelsVisibility (reuses the
  existing Visibility namespace). Added 8 new token structs to the PRELUDE,
  token_of, and the UIIR tag map.
- Cascade: the new namespaces reuse .small/.secondary/.words/.sentences/
  .standard/.none/.default, so converted the affected uniqueness-resolved
  modifiers to typed — foregroundColor, foregroundStyle, accentColor, tint,
  textInputAutocapitalization, headerProminence, badgeProminence. Every
  existing golden re-rendered byte-identical (behavior-preserving). green.
- Takeaway: the uniqueness→typed migration is now the standard move; each
  new token type is cheap but forces typing any older modifier that shared a
  name. Remaining token modifiers (textContentType, contentShape tokens,
  scenePadding, defersSystemGestures) follow the same recipe.

## Coverage iteration — SwiftUI text/scroll/dialog token modifiers

- Coverage before → after: SwiftUI implemented 326 → 332 (46.4% → 47.3%),
  verified 306 → 312 (43.6% → 44.4%). View +6. Others unchanged.
- Third typed-seam batch: textContentType (UITextContentType, 18 tokens),
  textSelectionAffinity, scrollInputBehavior, dialogSeverity, plus
  defaultHoverEffect and presentationDragIndicator reusing existing
  HoverEffect / Visibility namespaces. Four new token structs. No cascade
  this round — new tokens were unique or already resolved by typed peers.
  presubmit green.
- Cumulative SwiftUI arc this session: 40.0% → 47.3% implemented (+51
  modifiers), after breaking the token-collision plateau with the typed
  register_struct_method_typed seam.

## Coverage iteration — SwiftUI presentation/window metadata modifiers

- Coverage before → after: SwiftUI implemented 332 → 341 (47.3% → 48.6%),
  verified 312 → 321 (44.4% → 45.7%). View +9. stdlib/Foundation unchanged.
- Nine View modifiers, all recording real metadata onto the UIIR node
  (render-host semantics; hosts honor/ignore). Value passthroughs (no token):
  presentationCornerRadius (CGFloat), contentCaptureProtected (Bool),
  dialogPreventsAppTermination (Bool), listRowHoverEffectDisabled (Bool),
  typeSelectEquivalent (String), handlesExternalEvents (preferring:/allowing:
  [String] sets), navigationDocument (URL). Two token modifiers via the typed
  seam reusing existing namespaces: listRowHoverEffect (HoverEffect),
  sliderThumbVisibility (Visibility) — no new namespaces, no cascade.
- Verified by new presentation-metadata golden fixture; presubmit green.
- Note: stdlib (73.2%) and Foundation (75.3%) remaining gaps are dominated by
  unsafe-pointer / withUnsafeBytes / FormatStyle-token APIs that a headless
  interpreter can't implement with real behavior — SwiftUI modifier surface
  remains the highest-yield target.

## Coverage iteration — SwiftUI window/scene/container token modifiers

- Coverage before → after: SwiftUI implemented 341 → 349 (48.6% → 49.7%),
  verified 321 → 329 (45.7% → 46.9%). View +8.
- Eight View modifiers. Reusing existing token namespaces (typed seam):
  scenePadding + defersSystemGestures (Edge.Set), containerRelativeFrame
  (Axis.Set), pointerVisibility (Visibility). New WindowInteractionBehavior
  namespace (.automatic/.enabled/.disabled) wires four window modifiers:
  windowResizeBehavior, windowMinimizeBehavior, windowDismissBehavior,
  windowFullScreenBehavior — one new namespace, four modifiers.
- New-namespace recipe touchpoints confirmed: PRELUDE struct (lib.rs),
  token_of guard (values.rs), UIIR tag map (uiir.rs), typed registration
  (lib.rs), registered_keys test list. Verified by window-and-scene golden.
- Crossed the SwiftUI 49% impl mark; still on the modifier surface as the
  highest-yield target (stdlib/Foundation remainders are unsafe-pointer /
  format-token APIs infeasible for a headless interpreter).

## Coverage iteration — SwiftUI toolbar/margin modifiers (cross 50%)

- Coverage before → after: SwiftUI implemented 349 → 355 (49.7% → 50.6%),
  verified 329 → 335 (46.9% → 47.7%). View +6. **Crossed 50% implemented.**
- New ToolbarPlacement namespace (.automatic/.navigationBar/.tabBar/.bottomBar/
  .windowToolbar) unlocks four bar-targeted modifiers, each a leading token
  (Visibility or ColorScheme) plus a `for:` ToolbarPlacement selector — first
  multi-token modifiers where BOTH the positional and labeled args are typed
  token params. Plus value passthroughs contentMargins (CGFloat) and
  previewDevice (String).
- Cumulative session arc: SwiftUI 47.3% → 50.6% implemented (+23 modifiers)
  across three iterations, all green presubmit + golden-verified.

## Coverage iteration — SwiftUI token-namespace presentation modifiers

- Coverage before → after: SwiftUI implemented 355 → 367 (50.6% → 52.3%),
  verified 335 → 347 (47.7% → 49.4%). View +12. stdlib/Foundation unchanged.
- Twelve View modifiers via the typed seam. Ten new token namespaces (one
  per modifier, no cross-namespace collisions): navigationBarTitleDisplayMode
  (NavigationBarItemTitleDisplayMode), toolbarTitleDisplayMode
  (ToolbarTitleDisplayMode), toolbarRole (ToolbarRole), springLoadingBehavior
  (SpringLoadingBehavior), layoutDirectionBehavior (LayoutDirectionBehavior),
  textSelection (TextSelectability), previewLayout (PreviewLayout),
  previewInterfaceOrientation (InterfaceOrientation), symbolColorRenderingMode
  (SymbolColorRenderingMode), symbolVariableValueMode (SymbolVariableValueMode).
  Plus edgesIgnoringSafeArea (reuses Edge.Set) and backgroundStyle (Color value
  passthrough, no token).
- New-namespace recipe touchpoints: PRELUDE structs (lib.rs), token_of guard
  (values.rs), UIIR tag map (uiir.rs), typed registration (lib.rs), MODIFIER_FNS
  table (modifiers.rs), registered_keys_cover_v1_constructors expected list.
  Verified by new presentation-token-modifiers golden fixture; presubmit green.
- SwiftUI modifier surface remains highest-yield; stdlib (73.2%) and Foundation
  (75.3%) remainders dominated by unsafe-pointer / FormatStyle-token APIs a
  headless interpreter can't implement with real behavior.

## Coverage iteration — SwiftUI grid/presentation/material token modifiers

- Coverage before → after: SwiftUI implemented 367 → 379 (52.3% → 54.0%),
  verified 347 → 359 (49.4% → 51.1%). View +12. **Crossed 51% verified.**
- Twelve View modifiers via the typed seam. Reused namespaces: UnitPoint
  (defaultScrollAnchor, gridCellAnchor), HorizontalAlignment
  (gridColumnAlignment), Axis.Set (gridCellUnsizedAxes), Visibility
  (writingToolsAffordanceVisibility). Five new namespaces:
  presentationBackgroundInteraction, presentationCompactAdaptation
  (PresentationAdaptation .automatic/.none/.popover/.sheet/.fullScreenCover),
  scrollTargetBehavior (.viewAligned/.paging), materialActiveAppearance
  (.automatic/.active/.inactive), paletteSelectionEffect
  (.automatic/.symbolVariant/.custom). Plus Color value passthroughs
  listItemTint, listRowPlatterColor (no token). UnitPoint added as a first-class
  token namespace (10 anchors) reusable by future anchor-valued modifiers.
- Verified by grid-and-presentation-modifiers golden; presubmit green.
- Session arc: SwiftUI 50.6% → 54.0% implemented (+24 modifiers) over two
  iterations this session, all golden-verified with green presubmit.

## Coverage iteration — SwiftUI tab/search/toolbar token modifiers

- Coverage before → after: SwiftUI implemented 379 → 390 (54.0% → 55.6%),
  verified 359 → 370 (51.1% → 52.7%). View +11.
- Eleven View modifiers via the typed seam. Seven new namespaces:
  alternatingRowBackgrounds (.automatic/.enabled/.disabled), buttonSizing
  (.automatic/.fitted/.flexible), defaultAdaptableTabBarPlacement
  (AdaptableTabBarPlacement), tabBarMinimizeBehavior
  (.automatic/.onScrollDown/.onScrollUp/.never), searchPresentationToolbarBehavior
  (.automatic/.avoidHidingContent), searchToolbarBehavior (.automatic/.minimize),
  handGestureShortcut (.primaryAction). Multi-token: scrollEdgeEffectStyle
  (ScrollEdgeEffectStyle + for: Edge.Set), toolbarForegroundStyle (Color +
  for: ToolbarPlacement). Two no-arg markers: horizontalRadioGroupLayout,
  backgroundExtensionEffect.
- Verified by tab-search-toolbar-modifiers golden; presubmit green.
- Session arc: SwiftUI 50.6% → 55.6% implemented (+35 modifiers) over three
  iterations this session, all golden-verified with green presubmit.

## Coverage iteration — SwiftUI presentation/search/window token modifiers

- Coverage before → after: SwiftUI implemented 390 → 398 (55.6% → 56.7%),
  verified 370 → 378 (52.7% → 53.8%). View +8. stdlib/Foundation/SwiftData
  unchanged.
- Eight View modifiers via the typed seam. Four new namespaces:
  presentationContentInteraction (.automatic/.resizes/.scrolls),
  presentationSizing (.automatic/.fitted/.form/.page), searchDictationBehavior
  (TextInputDictationBehavior .automatic/.inactive),
  windowToolbarFullScreenVisibility (.automatic/.onHover). Reused namespace:
  windowResizeAnchor (UnitPoint). Multi-arg: scrollEdgeEffectHidden (leading
  Bool + `for:` Edge.Set). Value passthroughs: presentationBackground (Color),
  submitScope (Bool).
- Cascade fix: PresentationSizing.page collides with _ControlStyle.page, so the
  previously uniqueness-resolved tabViewStyle/indexViewStyle were converted to
  the typed seam (_ControlStyle) to keep `.page` resolving. All existing goldens
  re-render byte-identical.
- Verified by new presentation-window-modifiers golden fixture + a uiir
  serialization unit test. presubmit green.
- Blockers: remaining View modifiers are dominated by closure/binding-driven
  presentation (sheet/popover/alert/fullScreenCover), preference/anchor/geometry
  APIs, and effect/gesture modifiers that need more than a metadata record.

## Coverage iteration — SwiftData ModelContext change tracking + transactions

- Coverage before → after: SwiftData implemented 12 → 19 (10.5% → 16.7%),
  verified 10 → 17 (8.8% → 14.9%). ModelContext 5 → 12 (17.9% → 42.9%).
  stdlib/Foundation/SwiftUI unchanged.
- Seven ModelContext members, real behavior against the in-context change
  sets (inserted/tracked/deleted already maintained by insert/delete/save):
  hasChanges (bool), insertedModelsArray / changedModelsArray /
  deletedModelsArray (arrays of model objects), fetchCount(_:) (mirrors
  fetch's in-context semantics — pending-deleted excluded — by counting the
  same plan), rollback() (reverts dirty tracked objects to their last-flushed
  snapshot, un-marks pending deletes back into tracked, drops pending
  inserts), transaction(_:) (runs the closure then save()s atomically; on
  throw discards partial changes via rollback and re-propagates). Dirty
  detection compares current row_values to the Tracked snapshot; encoding
  errors treated as "not dirty" so a tracking query never spuriously throws.
- Registered hasChanges/*ModelsArray as contextual properties, fetchCount/
  rollback/transaction as method intrinsics. Verified end-to-end by new
  swiftdata_change_tracking golden (in-memory SQLite via the CLI's libsqlite3
  backing). presubmit green (exit 0).
- Shift from SwiftUI token modifiers (diminishing — remaining need
  closures/bindings) to SwiftData, the lowest-coverage framework (was 10.5%),
  where the existing change-tracking state made real-behavior wins cheap.
- Blockers/next: remaining ModelContext members split into PersistentIdentifier
  plumbing (registeredModel, model(for:)), history/undo (deleteHistory,
  fetchHistory, undoManager), and notification hooks (willSave/didSave). Schema
  and PersistentModel sections (0%) need @Model macro introspection surface.

## Coverage iteration — SwiftData config & fetch-descriptor properties

- Coverage before → after: SwiftData implemented 19 → 25 (16.7% → 21.9%),
  verified 17 → 23 (14.9% → 20.2%). ModelConfiguration 1 → 3 (18.8%),
  FetchDescriptor 1 → 5 (11.1% → 55.6%).
- Six value-type property reads via contextual properties, each faithfully
  returning init/mutation state: ModelConfiguration.isStoredInMemoryOnly,
  ModelConfiguration.name; FetchDescriptor.fetchLimit, .fetchOffset, .sortBy,
  .predicate (predicate value now retained on the descriptor object).
- Real behavior added: fetchOffset now paginates the SELECT — select_sql emits
  `LIMIT n OFFSET m`, or `LIMIT -1 OFFSET m` when only an offset is set (SQLite
  needs a LIMIT before OFFSET). Verified end-to-end: offset+limit page over a
  pages-sorted table returns the correct middle slice.
- Verified by the extended swiftdata_change_tracking golden + a new select_sql
  pagination unit test. presubmit green (exit 0).
- Session arc: SwiftData 10.5% → 21.9% implemented (+13 members) over two
  iterations, all golden-verified.
- Next: ModelConfiguration.allowsSave/url/schema (need save enforcement / store
  URL plumbing), ModelContainer.configurations/schema/deleteAllData, and the
  0%-coverage Schema/PersistentModel/PersistentIdentifier sections (need @Model
  macro introspection surface).

## Coverage iteration — SwiftData ModelContainer schema & deleteAllData

- Coverage before → after: SwiftData implemented 25 → 27 (21.9% → 23.7%),
  verified 23 → 25 (20.2% → 21.9%). ModelContainer 2 → 4 (25.0% → 50.0%).
- ModelContainer.schema (contextual property) returns a lightweight Schema
  value with the container's entity type names in registration order.
  ModelContainer.deleteAllData() runs one `DELETE FROM <table>` per model type
  and resets the tracking sets of every context on the shared connection, so a
  later fetch/fetchCount sees the emptied store (verified: count 2 → 0).
- Verified by the extended swiftdata_change_tracking golden. presubmit green.
- Session arc: SwiftData 10.5% → 23.7% implemented (+15 members) over three
  iterations, all golden-verified with green presubmit.

## Coverage iteration — SwiftUI effect & dialog value-passthrough modifiers

- Coverage before → after: SwiftUI implemented 398 → 409 (56.7% → 58.3%),
  verified 378 → 389 (53.8% → 55.4%). +11 View modifiers, all golden-verified.
- Eleven value-passthrough View modifiers (no leading-dot token, so no
  install-time typing needed): luminanceToAlpha (no-arg filter),
  rotation3DEffect (Angle + axis: tuple), keyboardShortcut (KeyEquivalent),
  containerShape (nested shape, like clipShape), dialogIcon (nested Image),
  fileDialogConfirmationLabel/CustomizationID/Message (String),
  fileDialogImportsUnresolvedAliases (Bool), fileDialogDefaultDirectory (URL),
  toolbarItemHidden (Bool). Each records its real value onto the UIIR node
  (verified in the new effects-and-dialogs golden — axis tuple, Angle, nested
  Circle()/Image, URL all serialize correctly).
- Mechanism: added `modifier!` defs + MODIFIER_FNS entries (auto-registers the
  `View.<name>` coverage keys) and updated the hardcoded expected-keys list in
  registered_keys_cover_v1_constructors. New tests/swiftui-fixtures fixture +
  regenerated .uiir.json golden. presubmit green (incl. wasm smoke).
- Next: remaining View modifiers increasingly need closures/bindings/namespaces
  (visualEffect, transaction, onGeometryChange, matchedGeometryEffect,
  searchScopes, sheet/popover/alert). Token modifiers (glassEffect,
  writingDirection, presentationDetents, symbolEffect) need new token structs +
  install typing. SwiftData Schema/PersistentModel (0%) still needs @Model
  macro introspection.

## Coverage iteration — SwiftUI search/status-bar/margin/glass modifiers

- Coverage before → after: SwiftUI implemented 409 → 413 (58.3% → 58.8%),
  verified 389 → 393 (55.4% → 56.0%). +4 View modifiers, golden-verified.
- searchCompletion (String value), statusBar(hidden:) (Bool),
  listSectionMargins (Edge.Set token + CGFloat, typed in install like padding),
  glassEffect (new `Glass` token struct .regular/.clear/.identity + nested
  `in:` shape view). Each records the real value onto the UIIR node — verified
  in the new search-and-margins golden (Edge.Set resolves to
  {"$":"edge","name":"horizontal"}; glass token + Capsule() nested shape).
- Added a `Glass` prelude token struct + two typed install registrations so the
  leading-dot args resolve against Edge.Set / Glass. Updated the hardcoded
  expected-keys list. presubmit green (incl. wasm smoke).
- Session arc: SwiftUI 56.7% → 58.8% implemented (+15 modifiers) over two
  iterations, all golden-verified.

## Coverage iteration — Foundation IndexSet Collection index conformance

- Coverage before → after: Foundation implemented 463 → 467 (75.3% → 75.9%),
  verified 461 → 465 (75.0% → 75.6%). IndexSet 29 → 33 impl (32 verified,
  76.2%).
- Real Collection conformance over the opaque `IndexSet.Index` (a 0-based
  position into the sorted members): startIndex (0), endIndex (count),
  index(after:)/index(before:) (±1 position step, label-sensitive intrinsic),
  indexRange(in: Range) (partition_point maps an integer range to the half-open
  position range covering its members), and subscript(position) in the core
  storage layer (reads the position-th sorted member from `_values`).
- Verified end-to-end by extending the foundation_indexset_ops golden:
  startIndex/endIndex, subscript at start/explicit/stepped positions, and
  indexRange(in: 3..<9) → 1..<2 over {2,5,9}. presubmit green.
- subscript is implemented (storage layer) but not a registry key, so it still
  reads "missing" in coverage; the 4 registry members are the counted gain.
- Next: IndexSet formIndex (needs dispatcher inout write-back like Array),
  hash(into:) (no Hasher surface yet). Foundation Date (55%) FormatStyle cases
  and URL (68%) remain the largest gaps.

## Coverage iteration — SwiftUI container/assistive/nav-transition modifiers

- Coverage before → after: SwiftUI implemented 413 → 418 (58.8% → 59.5%),
  verified 393 → 398 (56.0% → 56.7%). +5 View modifiers, golden-verified.
- containerCornerOffset (Edge.Set token + sizeToFit: Bool, typed in install),
  assistiveAccessNavigationIcon (String via systemImage:), sectionIndexLabel
  (String/Text label), hoverEffectGroup (no-arg group hint), navigationTransition
  (new `NavigationTransition` token struct .automatic/.slide, typed). Each
  records its real value onto the UIIR node — verified in the new
  container-and-nav golden.
- Session arc: SwiftUI 56.7% → 59.5% implemented (+20 modifiers) over three
  iterations; Foundation 75.3% → 75.9% (+4). All golden-verified, presubmit
  green each time.
- Blocker: remaining SwiftUI View modifiers overwhelmingly need closures/
  bindings/namespaces (sheet/popover/alert, matchedGeometryEffect, onKeyPress,
  visualEffect, transaction, searchScopes) — beyond value/token passthroughs.
  Swift Charts is not yet set up as a framework (no crate/inventory/scope);
  standing it up is greenfield infra requiring SDK swiftinterface extraction.

## Coverage iteration — stdlib String/Substring indices+iterator; SwiftUI presentations

- **stdlib**: 374 → 376 (73.2% → 73.6%). Added `indices` (materialised array of
  `String.Index` over valid subscript positions; base-relative for Substring)
  and `makeIterator` (Character array for explicit iteration) on both String
  and Substring. String 29→30, Substring 20→21. Golden
  `stdlib_string_indices_iterator` verifies iteration + subscripting; the
  Substring `indices.first == startIndex` invariant holds in base coordinates.
- **SwiftUI**: 418 → 421 impl (59.5% → 60.0%), 398 → 401 verified (56.7% →
  57.1%). First slice of the presentation-modifier family: `.sheet`,
  `.fullScreenCover`, `.popover`. Architecture designed via paseo-advisor
  (fable-5, medium) — verdict: the ADR-0013 "portal patch op" tripwire
  dissolves; the NavigationStack gated-subtree machinery already suffices.
- Design (ADR-0019): the modifier captures a **deferred** `_presentations`
  record (gating `Binding` + `@ViewBuilder` content closure + optional
  `onDismiss`) instead of a serialized `_Modifier`. A new session render pass
  (`presentation_node`, peer of `nav_stack_node`) reads the binding; when open
  it evaluates the content closure fresh and appends an in-tree `Presentation`
  child node (one node kind, `style` arg covers all styles). A host `dismiss`
  event writes the binding back to closed and fires `onDismiss`; programmatic
  close (state→false inside the sheet) drops the node on the next render for
  free. No new patch op, zero golden churn (internal `_`-fields never
  serialize). Goldens `sheet` (open→dismiss→onDismiss fires) and
  `presentation-styles` (fullScreenCover + popover) verify insert/remove
  patches.
- **Known fidelity gap (named degraded tier)**: `onDismiss` fires on a host
  `dismiss` event but not on programmatic close — SwiftUI fires on any
  dismissal. Closing it needs per-node presented-state tracking across renders;
  deferred until a fixture demands it (recorded in ADR-0019).
- Deferred siblings per advisor: `.alert`/`.confirmationDialog` (same node kind
  + title/message + auto-dismiss-on-action — the natural next slice),
  `@Namespace`/`matchedGeometryEffect` (identity token, no morph),
  `.onKeyPress` (needs a handled-flag on the dispatch response), `.visualEffect`
  (needs host geometry feedback), `.transaction` (animation-hint tier only).
- presubmit green (incl. wasm smoke + website checks); website coverage JSON
  regenerated.

## Coverage iteration — SwiftUI alert/confirmationDialog presentations

- **SwiftUI**: 421 → 423 impl (60.0% → 60.3%), 401 → 403 verified (57.1% →
  57.4%). Second presentation slice (ADR-0019): `.alert` and
  `.confirmationDialog`. A title string gates a `@ViewBuilder` `actions` closure
  (buttons) on a `Binding<Bool>`, with an optional `message:` closure rendered
  into a `message` arg. Reuses the `_presentations`/`Presentation`-node
  machinery; adds `title`/`message` args on the node.
- **Auto-dismiss on action** (SwiftUI semantics): tapping any button inside an
  alert/confirmationDialog closes it — implemented via `alert_ancestor_binding`
  (nearest enclosing alert Presentation) writing the gate closed after the
  action closure runs. Goldens `alert` (title+message, OK increments then
  dismisses) and `confirmation-dialog` (two actions, second sets choice then
  dismisses) verify insert/setText/remove patches.
- **Parser limitation surfaced**: the canonical two-trailing-closure form
  (`} message: {`) is not parseable (multi-trailing-closure unsupported, same
  gap AsyncImage's `} placeholder: {` hit). Workaround: pass `message:`
  explicitly in parens. Supporting multi-trailing-closure in the parser is the
  next high-leverage lever (unlocks AsyncImage placeholder form + natural alert
  syntax) but is a broader parser change.
- Session arc: SwiftUI 59.5% → 60.3% impl (+5 presentation modifiers) across two
  slices; stdlib 73.2% → 73.6%. presubmit green each time (incl. wasm smoke +
  website checks); coverage JSON regenerated.

## Coverage iteration — stdlib String/Substring initializers

- **stdlib**: 376 → 382 (73.6% → 74.8%). String 30→33 (56.6%→62.3%),
  Substring 21→24 (63.6%→72.7%). Added three real builtin constructors in
  `interp.rs` `builtin_ctor_table`:
  - `String(repeating: <String>, count: Int)` — repeat unit `count` times
    (negative count traps).
  - `String(_ value: BinaryInteger, radix: Int, uppercase: Bool = false)` —
    base 2…36 formatting via `int_to_radix_string` (negative → `-` + magnitude,
    matches Swift; unit-tested incl. `i128::MIN`).
  - `Substring(_:)` — full-range view over a String/Substring; `Substring()`
    empty. `ctor_string` delegates single-arg/`describing:` forms to
    `ctor_conversion`, and returns `Ok(None)` for unrecognised shapes so
    Foundation's `String(data:encoding:)`/`contentsOfFile:` free fn still wins.
- Marked `subscript`, `init`, `~=` as `core_members` for String and Substring
  in `frameworks/stdlib/scope.toml` — these are genuinely interpreter-level
  (subscript dispatch, builtin ctor, `~=` free-fn pattern match all verified
  working) but never surface in `registered_keys.txt`.
- Goldens: new `stdlib_string_init` (repeating/radix/uppercase/scalar/`~=`) and
  extended `stdlib_substring` (`Substring(_:)` from String/Substring/empty +
  `~=`). presubmit green (fmt + clippy + tests + wasm smoke + website checks);
  coverage JSON regenerated and drift-check clean.

## Coverage iteration — stdlib String char-view + small-collection ctors

- **stdlib**: 382 → 388 (74.8% → 75.9%). Two slices:
  - String character-view/contiguity parity with Substring: `characters`
    (returns self), `isContiguousUTF8` (always true), `makeContiguousUTF8()`
    (no-op mutating). String 33→36 (67.9%). Registered in `tswift-std`; golden
    `stdlib_string_init` extended.
  - `ArraySlice(_:)`/`CollectionOfOne` init + subscript marked `core_members`
    (interpreter-level, already working). ArraySlice 18→19, CollectionOfOne
    8→9. Golden `stdlib_arrayslice` covers `ArraySlice(_:)`.
- Remaining stdlib gaps are dominated by unsafe-pointer / span APIs
  (`withUnsafeBufferPointer`, `span`, `mutableSpan`, `withContiguousStorage…`)
  and `Hasher`/`customMirror` reflection — low value in this runtime.
- presubmit green each slice; coverage JSON regenerated + drift-clean.

## Coverage iteration — SwiftUI nested-subtree modifiers

- **SwiftUI**: 423 → 428 impl (60.3% → 61.0%), 403 → 408 verified (57.4% →
  58.1%). View modifiers 300→305 verified. Five real modifiers, all
  golden-verified via new `inset-and-swipe` fixture:
  - `contentShape(_:)` — records a nested hit-test shape descriptor (serialized
    like `clipShape`/`containerShape`). Leading `ContentShapeKinds` form
    (`.contentShape(.dragPreview, shape)`) not modelled; common single-shape
    form recorded straight onto the node.
  - `swipeActions(edge:allowsFullSwipe:content:)` — row action buttons captured
    as a nested subtree via `compose_modifier` (lowered like `contextMenu`).
    `edge:`/`allowsFullSwipe:` config not modelled yet (buttons recorded).
  - `safeAreaInset(edge:alignment:spacing:content:)` + newer `safeAreaBar(...)`
    — content `@ViewBuilder` resolved into a nested subtree (like `overlay`);
    `edge` (typed against `Edge` so `.top`/`.bottom`/`.leading`/`.trailing`
    resolve) and optional `spacing` ride on the modifier. `alignment:` not
    modelled.
  - `inspector(isPresented:content:)` — reuses the sheet/popover presentation
    machinery: a `Binding<Bool>` gates a `@ViewBuilder` pane realized as a
    `Presentation` child node with `style: "inspector"`. Fixture keeps it closed
    (no pane renders), exercising the capture path.
- Registration: `modifier!`/custom fns + `MODIFIER_FNS` (auto-drives
  `View.<name>` coverage keys and `registered_keys.txt`); typed specs for
  `safeAreaInset`/`safeAreaBar` `edge:` in `install`; expected-keys test vec and
  registered_keys.txt regenerated.
- Advisor (fable-5, prior session) had already confirmed the presentation
  vertical-slice design; the presentation family (sheet/popover/alert/etc.) is
  DONE, so this slice picks up cheap adjacent nested-subtree + binding-gated
  modifiers on the same machinery.
- presubmit green (fmt + clippy + tests + wasm smoke + website checks); coverage
  JSON regenerated; HTML progress report refreshed.

## Coverage iteration — SwiftUI token/effect passthrough modifiers

- **SwiftUI**: 428 → 433 impl (61.0% → 61.7%), 408 → 413 verified (58.1% →
  58.8%). Five real modifiers on the proven `modifier!` + `MODIFIER_FNS` +
  typed-`install` machinery, all golden-verified via new `symbol-and-detents`
  fixture:
  - `symbolEffect(_:)` — SF Symbol animation token (`SymbolEffect`:
    `.bounce`/`.pulse`/`.wiggle`/…). Tagged token `{"$":"symbolEffect",…}`.
  - `sensoryFeedback(_:trigger:)` — haptic/audio token (`SensoryFeedback`) plus
    a `trigger:` value passthrough (any Equatable). Typed positional token +
    labeled `trigger`.
  - `presentationDetents(_:)` — `[PresentationDetent]` token array
    (`[.medium, .large]`); leading-dot presets resolve in the array literal via
    the `[PresentationDetent]` typed param (precedent: `[GridItem]`).
  - `transformEffect(_:)` / `projectionEffect(_:)` — geometry-effect value
    passthroughs (recorded straight onto the node, no token).
- Recipe touched 4 seams: PRELUDE token structs (lib.rs), `token_of` allowlist
  (values.rs), `write_value` tag map (uiir.rs), typed `install` registrations.
  Expected-keys test vec + `registered_keys.txt` regenerated.
- **Collision surfaced**: `.success`/`.failure` SensoryFeedback presets collide
  with the builtin `Result` enum cases (interp.rs `register_builtin_result`) and
  degrade to a bare token string; the fixture uses `.selection`. Documented in
  the install comment.
- **`String.write(_:)` (TextOutputStream) rejected** — collides with
  Foundation's label-blind `String.write(to:)`/`write(toFile:)` intrinsic (same
  `"write"` registry key), which shadows the mutating append. Not cleanly
  separable without label-aware intrinsic dispatch; reverted.
- Remaining stdlib tail is dominated by unsafe-pointer/span/reflection/SIMD
  (`withUnsafeBufferPointer`, `span`, `mutableSpan`, `customMirror`, `hash`,
  `pointwiseMin/Max`) — low value in this runtime.
- presubmit green (fmt + clippy + tests + wasm smoke + website checks); coverage
  JSON regenerated + drift-clean; HTML progress report refreshed.

## Coverage iteration — SwiftUI window/scene style + touchbar modifiers

- **SwiftUI**: 433 → 438 impl (61.7% → 62.4%), 413 → 418 verified (58.8% →
  59.5%). Five real modifiers on the proven `modifier!` + `MODIFIER_FNS` +
  typed-`install` machinery, golden-verified via new `window-style-and-touchbar`
  fixture:
  - `presentedWindowStyle(_:)` — new `WindowStyle` token namespace
    (`.automatic`/`.plain`/`.hinted`/`.volumetric`).
  - `presentedWindowToolbarStyle(_:)` — new `WindowToolbarStyle` namespace
    (`.automatic`/`.expanded`/`.unified`/`.unifiedCompact`).
  - `typesettingLanguage(_:)` — new `TypesettingLanguage` namespace
    (`.automatic`; `.explicit(_)` builder not modelled).
  - `digitalCrownAccessory(_:)` — reuses the shared `Visibility` namespace
    (`.visible`/`.hidden`/`.automatic`).
  - `touchBarItemPrincipal(_:)` — plain `Bool` toggle (no token).
- Recipe touched 4 seams: PRELUDE token structs (lib.rs), `token_of` allowlist
  (values.rs), `write_value` tag map (uiir.rs), typed `install` registrations.
  Expected-keys test vec + `registered_keys.txt` regenerated.
- **Collision fixed**: adding `WindowStyle.plain` made the leading-dot `.plain`
  non-unique, so the four untyped `_ControlStyle` style modifiers that relied on
  global token uniqueness (`buttonStyle`/`listStyle`/`textFieldStyle`/
  `textEditorStyle`) degraded to bare strings (surfaced by the `more-styles`/
  `styling` goldens). Typed all four against `_ControlStyle` so `.plain` resolves
  contextually — the established issue-#203 pattern. `.automatic` was already
  non-unique, so its new occurrences add no regression.
- presubmit green (fmt + clippy + tests + wasm smoke + website checks); coverage
  JSON regenerated; HTML progress report refreshed.

## Coverage iteration — Swift Charts stood up (ADR-0020)

- **Swift Charts**: unregistered → **16/19 impl+verified (84.2%)** of the
  stage-1 tracked surface. New first-class framework: `frameworks.toml`
  `[charts]` entry, inventory generated from the iOS SDK
  `Charts.swiftmodule/*.swiftinterface` (61 types), `frameworks/charts/scope.toml`
  (tiers C1–C3 + honest `[out_of_scope]`), and a `registered_keys.txt` dumped by
  a new `dump_charts_registered_keys` test.
- **Design (advisor-confirmed)**: Charts marks *are* SwiftUI views — the weakest
  sufficient requirement is registering mark constructors that produce ordinary
  `view_value` leaf nodes under a `Chart` container, not a new render subsystem
  or a separate crate. Implemented as a `charts` module inside `tswift-swiftui`
  so marks compose with the existing modifier pipeline for free. ADR-0020.
- **Runtime**: `Chart { … }` (static, `collect_children`) and
  `Chart(data, id:) { d in … }` (data-driven, `keyed_rows` — sugar for a keyed
  `ForEach` of marks). Seven marks (`BarMark`/`LineMark`/`PointMark`/`AreaMark`/
  `RuleMark`/`RectangleMark`/`SectorMark`) record their plotted channels (`x`,
  `y`, `xStart`, `yEnd`, `angle`, `width`, `height`, `innerRadius`,
  `outerRadius`, `stacking`) as node args; unrecognized labels dropped.
- **Channel types** (SwiftUI PRELUDE, GridItem precedent):
  `PlottableValue<Value>.value(_:_:)` → `{"$":"plottable",label,value}` (value
  stored dynamically); `MarkDimension.automatic/.fixed/.ratio/.inset` →
  `{"$":"markDimension",kind,value}`; `MarkStackingMethod`/`InterpolationMethod`
  token structs (token_of allowlist) → tagged tokens. Leading-dot resolution via
  typed mark params (`x: PlottableValue`, `width: MarkDimension`,
  `stacking: MarkStackingMethod`) — issue #203 pattern.
- **Gotcha**: `MarkDimension.automatic` as a *computed* `static var` failed
  typed-param leading-dot resolution (`.automatic` unresolved); switching it to a
  `static let` fixed it (stored statics resolve, computed ones don't).
- **Fidelity tier named honestly** (ADR-0020): channels-recorded, host-drawn —
  no scale-domain/range inference, axis/legend layout, data binning, or mark
  stacking. Out-of-scope: axes/legends/scales, ChartProxy geometry read-back,
  scrollable/3-D charts, annotations, symbol shapes, vectorized plot content.
- Golden-verified via new `charts` render fixture (`tswift swiftui render`);
  every implemented member exercised. Remaining 3 "missing" = `Chart.body`
  (container has no Swift body), MarkDimension literal-init, `==` operator —
  all honest stage-1 gaps.
- Website: `charts.json` (auto-generated), status "at a glance" card, and a
  `status/charts.mdx` detail page. SwiftUI coverage unchanged (62.4%/59.5% — the
  marks are filtered out of the swiftui registry dump).
- presubmit green (fmt + clippy + tests + wasm smoke + website checks);
  coverage JSON regenerated + drift-clean; HTML progress report refreshed.

## Coverage iteration — buffer-pointer access (stdlib +1.6 pts)

- **Stdlib 388 → 396 verified (75.9% → 77.5%)**. New members, all
  golden-verified via `stdlib_buffer_pointer` CLI fixture:
  - `withUnsafeBufferPointer(_:)` and `withContiguousStorageIfAvailable(_:)`
    on Array, ArraySlice, ContiguousArray. The elements are materialized into
    a contiguous `Array` buffer value (a RandomAccessCollection, like
    `UnsafeBufferPointer`) and passed to the body closure; `count`/subscript/
    iteration/`reduce` resolve through the normal collection intrinsics.
    Contiguous storage always succeeds for arrays → `.some(result)`.
  - `withContiguousStorageIfAvailable(_:)` on String/Substring returns nil
    *without* invoking the closure — correct Swift behavior, since a String's
    backing store is UTF-8 bytes, not contiguous `Character` storage.
- **Fidelity tier (honest)**: read-only. Buffer mutation
  (`withUnsafeMutableBufferPointer`, `withContiguousMutableStorageIfAvailable`,
  `mutableSpan`) needs inout closure-param write-back and is deferred.
- Section moves: Array 68.8% → 75.0%, ArraySlice 65.5% → 72.4%,
  ContiguousArray 66.7% → 73.3%, String 67.9% → 69.8%, Substring 72.7% → 75.8%.
- Seam reuse: ArraySlice registers the shared `array` module fns; String/
  Substring share `install_shared_text_methods`. buffer_value uses
  `materialize_builtin_sequence` so it is receiver-agnostic.
- presubmit green (fmt + clippy + tests + wasm smoke + website checks);
  coverage JSON regenerated; HTML progress report refreshed.

## Coverage iteration — withExtendedLifetime (free fn +1)

- **Stdlib 396 → 397 verified (77.5% → 77.7%)**; free functions 29 → 30.
- `withExtendedLifetime(_ x:, _ body:)` — lifetime extension is a no-op in the
  interpreter (every value stays alive across a call), so it runs `body` and
  returns its result. The kept value is passed to the closure so both the
  `() -> R` and `(T) -> R` overloads work (extra args ignored by arg binding).
- Golden-verified via `stdlib_with_extended_lifetime` CLI fixture (zero-param
  body, value-taking body, and a Void body with a class side effect).
- presubmit green; coverage JSON regenerated; HTML report refreshed.

## Coverage iteration — SwiftUI accessibility modifiers (+7 verified)

- **SwiftUI 438→445 impl, 418→425 verified (59.5% → 60.5%)**; View section
  326→333 impl (67.3% → 68.8%). All golden-verified via the extended
  `accessibility` render fixture.
- Advisor (pi/fable-5) had already delivered the presentation-modifier design;
  on reading the code, presentation modifiers (sheet/popover/alert/confirmation-
  Dialog/fullScreenCover + dismiss event) are **already implemented** (ADR-0019,
  `session.rs::presentation_node`). Redundant advisor archived; pivoted to a
  clean, honest View-modifier slice with no new infra.
- New recording modifiers (all `modifier!` + MODIFIER_FNS table):
  - `accessibilityActivationPoint(_:)` — typed `UnitPoint` leading-dot token.
  - `accessibilityTextContentType(_:)` — new `AccessibilityTextContentType`
    token namespace (plain/console/fileSystem/messaging/narrative/sourceCode/
    spreadsheet/wordProcessing); typed + serialized as a tagged token
    (`token_of` allowlist + `uiir` tag map).
  - `accessibilityCustomContent(_:_:)` — label+value passthrough.
  - `accessibilityChartDescriptor(_:)` — opaque descriptor passthrough.
  - `accessibilityChildren`/`accessibilityRepresentation`/`accessibilityActions`
    — `@ViewBuilder`-composed nested subtrees, lowered via `compose_modifier`
    exactly like `overlay`/`background`.
- **Fidelity tier (honest)**: recorded-only. No on-device assistive tech in a
  headless runtime — the UIIR carries the semantic data; hosts honor or ignore.
- Updated the hardcoded `registered_keys_cover_v1_constructors` expectation.
- presubmit green (fmt + clippy + tests + wasm smoke + website checks);
  coverage JSON regenerated; HTML progress report refreshed.

## Coverage iteration — SwiftUI container/layout/bar-item modifiers (+4)

- **SwiftUI 445→449 impl, 425→429 verified (60.5% → 61.1%)**; View section
  333→337 (68.8% → 69.7%). Golden-verified via new
  `container-and-layout-modifiers` render fixture.
- New recording modifiers:
  - `containerBackground(_:for:)` / `containerBackground(for:){content}` —
    ShapeStyle/Color token or `@ViewBuilder` content (lowered like `background`)
    + new `ContainerBackgroundPlacement` token namespace (navigation/
    navigationSplitView/tabView/window), typed + tagged-token serialized.
  - `navigationBarItems(leading:trailing:)` — nested accessory views recorded
    like `tabItem` (each labeled arg expanded to a node).
  - `layoutValue(key:value:)` — LayoutValueKey metatype + value passthrough.
  - `previewContext(_:)` — value passthrough.
- **Fidelity tier (honest)**: recorded-only; hosts honor or ignore.
- presubmit green; coverage JSON regenerated; HTML report refreshed.

## Coverage iteration — SwiftUI searchable family (+5 verified)

- **SwiftUI 449→454 impl, 429→434 verified (61.1% → 61.8%)**; View section
  +5. Golden-verified via new `search-modifiers` fixture.
- Addresses the flagged `searchScopes` blocker plus the whole search family:
  - `searchable(text:placement:prompt:)` — snapshots the bound query string
    (read once via `wrappedValue`), records the `SearchFieldPlacement` token
    (new namespace: automatic/toolbar/sidebar/navigationBarDrawer) + prompt.
  - `searchScopes(_:activation:scopes:)` — scope-selection snapshot, new
    `SearchScopeActivation` token (automatic/onSearchPresentation/onTextEntry),
    and the `@ViewBuilder` scope list lowered to a child subtree (like overlay).
  - `searchSuggestions { }` — `@ViewBuilder` suggestion subtree.
  - `searchFocused(_:equals:)` / `searchSelection(_:)` — binding snapshots.
- New infra reused: `binding_snapshot` (read `wrappedValue`, else record as-is)
  + `compose_content` (shared `@ViewBuilder`→single-node lowering helper, cf.
  `tabItem`). `searchable`/`searchScopes` registered typed (token resolution) +
  in MODIFIER_FNS (coverage key). Two new token namespaces wired through
  `token_of` allowlist + uiir tag map.
- **Fidelity tier (honest)**: recorded-only — bindings read once, not wired for
  live two-way search; hosts honor or ignore the recorded metadata.
- presubmit green (fmt + clippy + tests + wasm smoke + website checks);
  coverage JSON regenerated; HTML progress report refreshed.

## Coverage iteration — String/Substring buffer access (+4 verified)

- **stdlib 397→401 impl/verified (77.7% → 78.5%)**; String 37→39
  (69.8% → 73.6%), Substring 25→27 (75.8% → 81.8%). Golden-verified via new
  `stdlib_string_buffer_access` fixture.
- New shared-text intrinsics (registered for both `String` and `Substring`):
  - `withUTF8(_:)` — invokes `body` with the UTF-8 code units as a contiguous
    `UInt8` buffer (modeled as an `Array`, like `UnsafeBufferPointer<UInt8>`),
    returning its result. Read-only tier (buffer mutation not modeled).
  - `withCString(_:)` — invokes `body` with the null-terminated bytes as a
    `CChar`/`Int8` buffer (trailing `0` included so the closure can walk to the
    terminator).
  - Both reuse the existing `call_closure` seam (cf. `Array.withUnsafeBuffer-
    Pointer`) plus a local `buffer_closure` trailing-closure extractor.
- **Dropped `write` from this slice**: `String.write` is already owned at
  runtime by Foundation's `write(to:)` (TextOutputStreamable → file URL), which
  overwrites any stdlib registration of the same name in the shared intrinsic
  map. Registering a second `write(_:)` (TextOutputStream append) would be dead
  code, so it was left out rather than faked.
- presubmit green (fmt + clippy + tests + wasm smoke + website checks);
  coverage JSON regenerated.

## Coverage iteration — SwiftUI closure-driven effect/scroll/event mods (+10)

- **SwiftUI 454→464 impl, 434→444 verified (61.8% → 63.2%)**; View section
  341 verified. Golden-verified via new `closure-effect-modifiers` fixture.
- Addresses the flagged `visualEffect` / `transaction` blockers plus a coherent
  family of closure-driven modifiers, all at an honest **recorded-only** tier:
  - `transaction { }`, `visualEffect { }`, `transformEnvironment(_:_:)`,
    `scrollTransition { }`, `onGeometryChange(for:of:action:)`,
    `onScrollGeometryChange(...)`, `onScrollPhaseChange { }`,
    `onScrollVisibilityChange(threshold:_:)`, `onPreferenceChange(_:perform:)`,
    `onModifierKeysChanged { }`.
- New `closure_modifier!` macro: appends a bare marker (so hosts know the
  listener/effect is present) and stashes the trailing closure under the same
  event key via the existing `attach_event` seam — closures never serialize.
  The closure's argument (`Transaction`/`GeometryProxy`/`ScrollGeometry`/
  preference value) is not synthesized by a headless runtime, so the body is
  not invoked; non-closure args (metatypes, key paths) are dropped from the
  marker. Registered through the `MODIFIER_FNS` table (coverage keys) and the
  hardcoded `registered_keys_cover_v1_constructors` expectation was updated.
- **Fidelity tier (honest)**: recorded-only — markers cross the UIIR boundary
  in order; no live effect/scroll/preference wiring. Hosts honor or ignore.
- presubmit green (fmt + clippy + tests + wasm smoke + website checks);
  coverage JSON regenerated.

## Coverage iteration — SwiftUI preference/phase/command modifiers (+5)

- **SwiftUI 464→469 impl, 444→449 verified (63.2% → 64.0%)**. Golden-verified
  via new `preference-and-command-modifiers` fixture.
- New recorded-only modifiers:
  - `transformPreference(_:_:)`, `phaseAnimator(_:content:)`,
    `onCommand(_:perform:)`, `onPasteCommand(of:perform:)` — reuse the
    `closure_modifier!` macro (bare marker + stashed trailing closure).
  - `preference(key:value:)` — carries no closure; records the `value` payload
    (hosts read the preference value). The `key:` metatype is not representable
    in the value model, so it is dropped (recorded-only tier).
- **Deliberately dropped** (args not constructible in a headless fixture, so
  cannot be golden-verified honestly): `anchorPreference`/
  `transformAnchorPreference` (need `Anchor.Source` `.bounds`, an unresolved
  leading-dot token) and `onReceive` (needs a Combine/Timer/NotificationCenter
  publisher, none modeled). Left as tripwires for when those types land.
- Updated the hardcoded `registered_keys_cover_v1_constructors` expectation.
- presubmit green; coverage JSON regenerated.

## Coverage iteration — @Namespace + matchedGeometryEffect (+2)

- **SwiftUI 469→471 impl, 449→451 verified (64.0% → 64.2%)**. Resolves the
  flagged `matchedGeometryEffect` blocker. Golden-verified via new
  `matched-geometry` fixture.
- New prelude infra: `@Namespace var ns` property wrapper whose `wrappedValue`
  is an opaque `NamespaceID` identity token (the runtime is headless — no
  layout engine to actually match geometry).
- New modifiers (custom fns):
  - `matchedGeometryEffect(id:in:properties:anchor:isSource:)` — records `id:`
    (Hashable) and `isSource:`; drops the `in:` namespace + `properties:`/
    `anchor:` (recorded-only tier).
  - `matchedTransitionSource(id:in:configuration:)` — records `id:`.
- Updated the hardcoded `registered_keys_cover_v1_constructors` expectation.
- **Fidelity tier (honest)**: recorded-only — geometry identity crosses the
  UIIR boundary; no on-device geometry matching/morph.
- presubmit green; coverage JSON regenerated.

## Coverage iteration — SwiftUI gesture/content modifiers (+5)

- **SwiftUI 471→476 impl, 451→456 verified (64.2% → 65.0%)**. Golden-verified
  via new `gesture-and-content-modifiers` fixture.
- New modifiers:
  - `highPriorityGesture(_:)` / `simultaneousGesture(_:)` — reuse the existing
    `modifier_gesture` lowering (TapGesture/LongPressGesture → onTapGesture/
    onLongPressGesture marker + handler), matching `.gesture(_:)`.
  - `renameAction(_:)` — recorded-only marker + stashed closure
    (`closure_modifier!`).
  - `toolbarTitleMenu { }` / `sectionActions { }` — new `viewbuilder_modifier!`
    macro lowers the trailing `@ViewBuilder` to a nested child subtree (like
    `tabItem`/`searchSuggestions`).
- Updated the hardcoded `registered_keys_cover_v1_constructors` expectation.
- presubmit green; coverage JSON regenerated.

## Coverage iteration — SwiftUI preference-value/ornament modifiers (+3)

- **SwiftUI 476→479 impl, 456→459 verified (65.0% → 65.4%)**. Golden-verified
  via new `preference-value-and-ornament` fixture.
- New modifiers:
  - `backgroundPreferenceValue(_:_:)` / `overlayPreferenceValue(_:_:)` — take a
    `(Value) -> View` transform; the preference `Value` is not computed by a
    headless runtime, so they record a bare marker + stashed closure
    (`closure_modifier!`) rather than fake empty content.
  - `ornament { }` — `viewbuilder_modifier!` lowers the trailing `@ViewBuilder`
    to a nested child subtree (labeled visibility/anchor/alignment dropped).
- **Deferred**: `scrollPosition`/`focused` need optional-`@State`/`@FocusState`
  infra (not yet modeled); left as tripwires.
- Updated the hardcoded `registered_keys_cover_v1_constructors` expectation.
- presubmit green; coverage JSON regenerated.
