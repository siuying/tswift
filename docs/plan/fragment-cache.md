# Plan: interpreter-owned fragment cache (ADR-0007)

Implements ADR-0007 — replace the per-interpolation `Box::leak` with an
interpreter-owned, append-only, source-keyed cache so a long-running native host
runs in bounded memory. This is the prerequisite the `TSwiftCore`/`TSwiftUI`
work pivoted to land first.

See: `docs/adr/0007-fragment-cache-reclaimable-interpolation-analyses.md`,
`CONTEXT.md` (Fragment leak / Fragment cache), and the leak site
`crates/tswift-core/src/interp.rs` `eval_interpolation` (~line 7233).

## Steps

1. **New module `crates/tswift-core/src/fragment_cache.rs`.**
   - `interp` is a single-file module (`mod interp;` in `lib.rs`). Add a sibling
     `mod fragment_cache;` to `lib.rs` (private) and `use crate::fragment_cache`
     from `interp.rs`.
   - `struct FragmentCache { entries: Vec<Box<Analysis>>, index: HashMap<String, usize> }`,
     `Default`.
   - `fn get_or_analyze(&mut self, src: &str) -> Result<&'static Analysis, AnalyzeError-ish>`:
     - hit (`index.get(src)`) → return the stored `'static` ref.
     - miss → `Analysis::analyze(src, "interpolation")`, validate `is_ok()`,
       `Box::new`, `push`, insert index, then
       `unsafe { &*(std::ptr::from_ref(&**entry)) }` transmuted to `'static`.
   - Document the **never-evict** invariant in-module as a soundness requirement
     (removing an entry while a `Node<'static>` points into it is UB). Reference
     `Node: Copy + no Drop` as the reason field drop-order is safe.
   - Keep the `unsafe` to this one module; add a module-level `// SAFETY:` block.

2. **Thread the cache into `Interpreter`.**
   - Add `fragment_cache: FragmentCache` field (init in `Interpreter::new`).
   - Rewrite `eval_interpolation` to call
     `self.fragment_cache.get_or_analyze(fragment)?` instead of
     `Box::leak(Box::new(analysis))`. Preserve the existing error messages
     ("interpolation parse error", "invalid interpolation `{fragment}`").

3. **Tests** (`crates/tswift-core/src/interp.rs` test module or alongside):
   - Re-rendering the same interpolation analyzes the fragment **once**: drive
     `eval_interpolation` (or a small program with a loop printing `"\(i)"`) and
     assert `fragment_cache.entries.len() == 1` for a single distinct fragment.
   - Distinct fragments grow the cache by their distinct count, not by eval count.
   - Reclamation: dropping the `Interpreter` drops the cache (a no-leak smoke
     test; if the repo has a leak-checking harness use it, otherwise assert
     `Drop` runs / entries count via a scoped block).
   - Existing interpolation tests (e.g. `string_interpolation_renders_expressions`,
     shorthand `$0` cases ~interp.rs:8725+) must stay green.

4. **Verify & commit.**
   - `scripts/presubmit` (fmt + clippy + test) must be green. Clippy will scrutinize
     the `unsafe`; keep the SAFETY comment precise.
   - Commit (conventional, `--no-gpg-sign`, no AI trailers), e.g.
     `perf(core): reclaim interpolation analyses via fragment cache` or
     `fix(core): bound interpolation memory with owned fragment cache`.

## Acceptance

- No `Box::leak` remains in `eval_interpolation`.
- A program that evaluates one distinct interpolation N times holds exactly one
  cached `Analysis`.
- Dropping the interpreter frees the cache (no cross-session growth).
- `scripts/presubmit` green; existing interpolation/shorthand tests pass.

## Then resume the pivot

Once landed, return to the deferred `TSwiftCore`/`TSwiftUI` design tree
(C-ABI shape, string-ownership/free contract, xcframework packaging, how
`TSwiftUI` drives `UiirRenderer`). Open branches are listed in the handoff doc.
