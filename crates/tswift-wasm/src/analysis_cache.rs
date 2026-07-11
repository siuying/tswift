//! Warm-start caching of the frontend [`Analysis`] keyed by program input.
//!
//! **This is runtime caching, not compilation.** tswift is an interpreter with
//! a pure-Rust frontend; there is no ahead-of-time codegen and nothing is
//! "compiled to wasm". This cache only lets a *re-submission of byte-identical
//! input* (a Studio re-run, an embed refresh, a "Run" pressed twice) skip the
//! lex/parse/sema pass and reuse the previously produced `Analysis`. The
//! interpreter still runs fresh every time — only the analyze phase is elided,
//! so program side effects (stdout, host calls) are unaffected. The cache is
//! therefore invisible except as a small latency reduction.
//!
//! ## Real ownership — the cache frees on eviction
//!
//! The cache owns each `Analysis` behind an [`Rc`] and hands out `Rc` clones.
//! An entry evicted by the LRU drops the cache's `Rc`; the backing AST is then
//! freed **as soon as no interpreter is still using it**. Crucially, the cache
//! does **not** `Box::leak`: total memory held by the cache is bounded by
//! [`CACHE_CAP`] retained programs, regardless of how many *distinct* programs
//! are submitted over the process lifetime. This fixes the earlier design where
//! every miss leaked a `&'static Analysis` and eviction orphaned (but never
//! reclaimed) the leak, so unique submissions grew memory without bound.
//!
//! ## Why an `Rc`, not a `&'static`
//!
//! The interpreter is built around `Node<'static>` cursors into the AST
//! ([`tswift_core::Interpreter::run`] takes `&'static Analysis`), and a
//! long-lived SwiftUI [`crate::swiftui`] session holds those cursors across
//! dispatch calls — so the AST genuinely must outlive each run. Rather than
//! leak to satisfy that, callers pass their `Rc<Analysis>` to
//! [`tswift_core::Interpreter::run_retaining`], which retains the `Rc` for the
//! interpreter's lifetime (a bounded, `FragmentCache`-style ownership model).
//! The cache's own `Rc` is then free to be evicted independently: a running or
//! session-held interpreter keeps its clone alive; the AST is reclaimed when
//! the last holder — cache entry or interpreter — drops. `tswift-wasm` stays
//! `#![forbid(unsafe_code)]`; the single `unsafe` deriving `'static` from the
//! retained `Rc` lives in `tswift-core` alongside `FragmentCache`.
//!
//! ## Collision safety
//!
//! Entries are keyed by a `DefaultHasher` digest of length-prefixed key
//! material, but a hit is only accepted after a full byte-for-byte comparison
//! of the stored key against the request — a hash collision degrades to a miss
//! (re-analyze), never a wrong-`Analysis` hit. The key material carries an
//! **entry-mode tag** (single vs. multi-file, `run` vs. SwiftUI compile) and,
//! for multi-file inputs, the ordered per-file `(path, source)` pairs — each
//! length-prefixed — so a module `[a, b]` can never alias the single source
//! `a + b`, nor a `run` submission alias a SwiftUI one.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use tswift_frontend::{Analysis, AnalyzeError, SourceFile};

/// Maximum number of distinct programs kept warm (LRU). Small on purpose: the
/// realistic reuse pattern is "the same one or two sources re-run repeatedly".
/// Because the cache owns each `Analysis` via `Rc` and frees on eviction, this
/// is a hard bound on the cache's retained memory (not merely a bound on the
/// count of leaks, as in the earlier `&'static` design).
const CACHE_CAP: usize = 4;

struct Entry {
    hash: u64,
    /// Full key material for collision-proof comparison (see module docs).
    key: String,
    analysis: Rc<Analysis>,
}

thread_local! {
    /// Most-recently-used at the back; oldest at the front (evicted first).
    static CACHE: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

/// Analyze a single-file `run` program, reusing a cached `Analysis` when
/// `source` (under `filename`) was analyzed before.
pub(crate) fn analyze_cached(source: &str, filename: &str) -> Result<Rc<Analysis>, AnalyzeError> {
    let key = build_key("run1", &[(filename, source)]);
    get_or_insert(&key, || Analysis::analyze(source, filename))
}

/// Analyze a multi-file `run` program, reusing a cached `Analysis` when the
/// exact ordered `files` (paths + contents) were analyzed before.
pub(crate) fn analyze_program_cached(files: &[SourceFile]) -> Result<Rc<Analysis>, AnalyzeError> {
    let pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|f| (f.path.as_str(), f.source.as_str()))
        .collect();
    let key = build_key("runM", &pairs);
    get_or_insert(&key, || Analysis::analyze_program(files))
}

/// Analyze under a caller-built cache `key` (see [`swiftui_single_key`] /
/// [`swiftui_program_key`]). The `analyze` closure runs only on a miss; on a
/// hit the cached `Rc` is cloned and returned without re-analyzing.
///
/// This lets the SwiftUI compile path key on its *structural* file boundaries
/// while analyzing a *merged* source (prelude + program), so the key can't be
/// reconstructed by concatenation alone.
pub(crate) fn analyze_keyed(
    key: String,
    analyze: impl FnOnce() -> Result<Analysis, AnalyzeError>,
) -> Result<Rc<Analysis>, AnalyzeError> {
    get_or_insert(&key, analyze)
}

/// Cache key for a single-source SwiftUI compile (entry mode `ui1`).
pub(crate) fn swiftui_single_key(source: &str) -> String {
    build_key("ui1", &[("main.swift", source)])
}

/// Cache key for a multi-file SwiftUI compile (entry mode `uiM`): the ordered
/// per-file `(path, contents)` pairs, so a module never aliases a single source
/// equal to its concatenation, nor a `run` submission, nor a single-file
/// SwiftUI compile.
pub(crate) fn swiftui_program_key(files: &[(String, String)]) -> String {
    let pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    build_key("uiM", &pairs)
}

fn get_or_insert(
    key: &str,
    analyze: impl FnOnce() -> Result<Analysis, AnalyzeError>,
) -> Result<Rc<Analysis>, AnalyzeError> {
    let hash = digest(key);
    if let Some(hit) = CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if let Some(idx) = cache.iter().position(|e| e.hash == hash && e.key == key) {
            // Promote to most-recently-used and return a shared clone.
            let entry = cache.remove(idx);
            let analysis = Rc::clone(&entry.analysis);
            cache.push(entry);
            Some(analysis)
        } else {
            None
        }
    }) {
        return Ok(hit);
    }

    // Miss: analyze once and record. The `Rc` is owned by the cache; on
    // eviction its clone count drops and the AST is freed once no interpreter
    // still retains it.
    let analysis = Rc::new(analyze()?);
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if cache.len() >= CACHE_CAP {
            // Dropping the front entry drops the cache's `Rc`; the backing AST
            // is reclaimed as soon as the last retaining interpreter drops.
            cache.remove(0);
        }
        cache.push(Entry {
            hash,
            key: key.to_string(),
            analysis: Rc::clone(&analysis),
        });
    });
    Ok(analysis)
}

fn digest(key: &str) -> u64 {
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    h.finish()
}

/// Build length-prefixed key material: an entry-mode `mode` tag followed by the
/// ordered per-file `(path, source)` pairs. Length-prefixing each field means
/// distinct inputs can never alias by concatenation (`"ab"+"c"` vs `"a"+"bc"`),
/// and the leading mode tag keeps the four entry points (`run` single/multi,
/// SwiftUI single/multi) in disjoint key spaces.
fn build_key(mode: &str, files: &[(&str, &str)]) -> String {
    let mut key = String::new();
    push_field(&mut key, mode);
    // Encode the file count so a 1-file multi input can't collide with a
    // single-file input that happens to share the mode-adjacent bytes.
    push_field(&mut key, &files.len().to_string());
    for (path, source) in files {
        push_field(&mut key, path);
        push_field(&mut key, source);
    }
    key
}

fn push_field(buf: &mut String, field: &str) {
    buf.push_str(&field.len().to_string());
    buf.push(':');
    buf.push_str(field);
    buf.push(';');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear() {
        CACHE.with(|c| c.borrow_mut().clear());
    }

    #[test]
    fn hit_returns_same_allocation() {
        clear();
        let a = analyze_cached("let x = 1", "main.swift").unwrap();
        let b = analyze_cached("let x = 1", "main.swift").unwrap();
        assert!(Rc::ptr_eq(&a, &b), "identical source must reuse Analysis");
    }

    #[test]
    fn distinct_source_is_a_miss() {
        clear();
        let a = analyze_cached("let x = 1", "main.swift").unwrap();
        let b = analyze_cached("let x = 2", "main.swift").unwrap();
        assert!(!Rc::ptr_eq(&a, &b), "different source must re-analyze");
    }

    #[test]
    fn filename_participates_in_key() {
        clear();
        let a = analyze_cached("let x = 1", "a.swift").unwrap();
        let b = analyze_cached("let x = 1", "b.swift").unwrap();
        assert!(!Rc::ptr_eq(&a, &b), "filename is part of the key");
    }

    #[test]
    fn lru_evicts_oldest_and_frees_it() {
        clear();
        let first = analyze_cached("let a = 0", "main.swift").unwrap();
        // The cache holds one `Rc`; `first` is the only other holder here.
        assert_eq!(Rc::strong_count(&first), 2, "cache + local holder");
        for i in 1..=CACHE_CAP {
            let _ = analyze_cached(&format!("let a = {i}"), "main.swift").unwrap();
        }
        // `first` was the oldest of CACHE_CAP+1 distinct programs → evicted, so
        // the cache dropped its `Rc`. Only our local clone remains.
        assert_eq!(
            Rc::strong_count(&first),
            1,
            "evicted entry must be dropped by the cache (real free, not leak)"
        );
        let again = analyze_cached("let a = 0", "main.swift").unwrap();
        assert!(
            !Rc::ptr_eq(&first, &again),
            "evicted entry must be re-analyzed to a fresh Analysis"
        );
    }

    #[test]
    fn multi_file_key_is_order_sensitive() {
        clear();
        let f1 = SourceFile::new("a.swift", "struct A {}");
        let f2 = SourceFile::new("main.swift", "print(\"hi\")");
        let a = analyze_program_cached(&[f1.clone(), f2.clone()]).unwrap();
        let b = analyze_program_cached(&[f1.clone(), f2.clone()]).unwrap();
        assert!(Rc::ptr_eq(&a, &b), "same ordered files reuse Analysis");
        let c = analyze_program_cached(&[f2, f1]).unwrap();
        assert!(!Rc::ptr_eq(&a, &c), "reordered files must re-analyze");
    }

    #[test]
    fn multi_file_module_never_aliases_concatenated_single_source() {
        clear();
        // A two-file module whose contents concatenate to the same bytes as a
        // single-source `run` submission must NOT share a cache entry: the key
        // carries file boundaries + an entry-mode tag.
        let files = [
            ("a.swift".to_string(), "let a = 1".to_string()),
            ("main.swift".to_string(), "let b = 2".to_string()),
        ];
        let program_key = swiftui_program_key(&files);
        let single_key = swiftui_single_key("let a = 1\nlet b = 2");
        assert_ne!(
            program_key, single_key,
            "module boundaries must not collapse into concatenated source"
        );
    }

    #[test]
    fn entry_mode_separates_run_and_swiftui_keys() {
        clear();
        // The same source analyzed via the `run` path and the SwiftUI compile
        // path (which prepends a different prelude) must land in disjoint key
        // spaces, or one would serve the other's Analysis.
        let run_key = build_key("run1", &[("main.swift", "let x = 1")]);
        let ui_key = swiftui_single_key("let x = 1");
        assert_ne!(run_key, ui_key, "run vs SwiftUI must not share a key");
    }
}
