//! Interpreter-owned, append-only cache of string-interpolation fragment
//! analyses (ADR-0007).
//!
//! String interpolation (`"\(expr)"`) parses each `expr` fragment into its own
//! [`Analysis`]. ADR-0003 made the interpreter operate on `Node<'static>` and
//! leaked one `Analysis` per fragment *per evaluation* via `Box::leak`. That is
//! bounded for a run-once CLI but grows without bound in a long-running,
//! repeatedly-recompiling host (`TSwiftUI`). This cache replaces the leak with
//! interpreter-owned storage that is reclaimed when the interpreter drops.
//!
//! ## SAFETY
//!
//! This module hands out `&'static Analysis` references that do **not** actually
//! live for the process lifetime — they live as long as the owning
//! [`FragmentCache`] (a field of the interpreter). The transmute to `'static` is
//! sound because of three invariants, all of which must be preserved by future
//! edits:
//!
//! 1. **Stable addresses.** Each `Analysis` is stored behind a `Box`, so its heap
//!    address never moves when the backing `Vec` reallocates — pushing later
//!    fragments only moves box pointers, never the `Analysis` itself. A
//!    `&'static Analysis` handed out earlier therefore stays valid.
//!
//! 2. **Never evict.** The cache is append-only and never removes an entry. This
//!    is a *requirement*, not a convenience: dropping an `Analysis` while a
//!    `Node<'static>` cursor still points into it would be use-after-free. The
//!    set of distinct fragments is bounded by the program text, so the cache
//!    plateaus rather than growing unbounded.
//!
//! 3. **No `Drop` on cursors.** `Node<'a>` is `Copy` with no `Drop`
//!    implementation, so stored `Node<'static>` cursors never dereference their
//!    analysis on drop. Interpreter field drop-order is therefore irrelevant: a
//!    cursor outliving (in drop-order) the cache it points into never touches the
//!    freed memory.

use std::collections::HashMap;

use tswift_frontend::Analysis;

/// Append-only, source-keyed cache of interpolation-fragment analyses.
///
/// See the module-level `SAFETY` block for the invariants that make the
/// `'static` references returned by [`get_or_analyze`](FragmentCache::get_or_analyze)
/// sound.
#[derive(Default)]
pub(crate) struct FragmentCache {
    /// Boxed analyses, each at a stable heap address. Never shrinks.
    ///
    /// The `Box` is load-bearing, not redundant: it keeps each `Analysis` at a
    /// fixed address when the `Vec` reallocates, so the `&'static` references
    /// handed out earlier stay valid. `Vec<Analysis>` would move the analyses on
    /// growth and dangle those references — see the module `SAFETY` block.
    #[allow(clippy::vec_box)]
    entries: Vec<Box<Analysis>>,
    /// Fragment source text → index into `entries`.
    index: HashMap<String, usize>,
}

/// Why analyzing an interpolation fragment failed.
pub(crate) enum FragmentError {
    /// The fragment did not parse.
    Parse(String),
    /// The fragment parsed but failed analysis (`!analysis.is_ok()`).
    Invalid,
}

impl FragmentCache {
    /// Return the analysis for `src`, analyzing and caching it on first sight.
    ///
    /// On a cache hit the stored `'static` reference is returned without
    /// re-analyzing. On a miss the fragment is analyzed once, boxed, and stored;
    /// the returned reference points into that owned storage (see the
    /// module-level `SAFETY` block).
    pub(crate) fn get_or_analyze(&mut self, src: &str) -> Result<&'static Analysis, FragmentError> {
        if let Some(&idx) = self.index.get(src) {
            return Ok(Self::as_static(&self.entries[idx]));
        }
        let analysis = Analysis::analyze(src, "interpolation")
            .map_err(|e| FragmentError::Parse(e.to_string()))?;
        if !analysis.is_ok() {
            return Err(FragmentError::Invalid);
        }
        let idx = self.entries.len();
        self.entries.push(Box::new(analysis));
        self.index.insert(src.to_string(), idx);
        Ok(Self::as_static(&self.entries[idx]))
    }

    /// Number of distinct fragments currently cached.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Transmute a boxed analysis's borrow to `'static`.
    ///
    /// SAFETY: the `Box` keeps the `Analysis` at a stable address for as long as
    /// the cache lives, the cache never evicts, and `Node` cursors carry no
    /// `Drop`. See the module-level `SAFETY` block for the full argument.
    fn as_static(entry: &Analysis) -> &'static Analysis {
        unsafe { &*std::ptr::from_ref::<Analysis>(entry) }
    }
}
