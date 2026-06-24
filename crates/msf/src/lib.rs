//! Safe wrapper over [`msf_sys`].
//!
//! This crate is the **only** place that dereferences msf's raw pointers. It
//! upholds the one invariant msf's C ABI requires of a consumer:
//!
//! > The arena-allocated AST and `TypeInfo` graph live exactly as long as the
//! > owning analysis result. Never read a node after the result is freed.
//!
//! We encode that invariant in the type system: [`Analysis`] owns the
//! `*mut MSFResult` and frees it on `Drop`; every [`Node`] borrows the
//! `Analysis` (`Node<'a>`), so the borrow checker forbids using a node past the
//! analysis it came from. Above this crate, no `unsafe` is needed to walk an AST.

use std::ffi::{CStr, CString};

mod kind;
pub use kind::NodeKind;

/// An owned msf analysis result: the typed, immutable AST plus diagnostics for
/// one Swift source file. Frees the underlying `MSFResult` on drop.
pub struct Analysis {
    raw: *mut msf_sys::MSFResult,
}

impl Analysis {
    /// Tokenize, parse, and type-resolve `source`. `filename` is used only in
    /// diagnostics. Returns `Err` if the inputs contain interior NUL bytes, or
    /// if msf reports an allocation failure (its only hard-failure mode).
    pub fn analyze(source: &str, filename: &str) -> Result<Analysis, AnalyzeError> {
        let code = CString::new(source).map_err(|_| AnalyzeError::InteriorNul)?;
        let fname = CString::new(filename).map_err(|_| AnalyzeError::InteriorNul)?;
        // SAFETY: both pointers are valid, NUL-terminated C strings that outlive
        // the call. msf copies what it needs out of them before returning.
        let raw = unsafe { msf_sys::msf_analyze(code.as_ptr(), fname.as_ptr()) };
        if raw.is_null() {
            return Err(AnalyzeError::Allocation);
        }
        Ok(Analysis { raw })
    }

    /// The root `source_file` node of the AST.
    pub fn root(&self) -> Node<'_> {
        // SAFETY: `self.raw` is a live result; `msf_root` returns a node owned by
        // it. The returned `Node` borrows `self`, tying its lifetime to ours.
        let ptr = unsafe { msf_sys::msf_root(self.raw) };
        Node {
            ptr,
            analysis: self,
        }
    }

    /// Semantic/syntactic errors produced during analysis, in source order.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        // SAFETY: `self.raw` is live for the duration of the call.
        let count = unsafe { msf_sys::msf_error_count(self.raw) };
        (0..count)
            .map(|i| {
                // SAFETY: `i` is in `0..count`; msf returns a static-lifetime
                // (result-owned) C string and packed line/col integers.
                let msg = unsafe {
                    let c = msf_sys::msf_error_message(self.raw, i);
                    cstr_to_string(c)
                };
                let line = unsafe { msf_sys::msf_error_line(self.raw, i) };
                let col = unsafe { msf_sys::msf_error_col(self.raw, i) };
                Diagnostic {
                    message: msg,
                    line,
                    col,
                }
            })
            .collect()
    }

    /// Returns `true` if analysis produced no errors.
    pub fn is_ok(&self) -> bool {
        // SAFETY: `self.raw` is live.
        unsafe { msf_sys::msf_error_count(self.raw) == 0 }
    }

    /// The owned source descriptor (used to resolve token text).
    fn source(&self) -> *const msf_sys::Source {
        // SAFETY: `self.raw` is live; the source is owned by it.
        unsafe { msf_sys::msf_source(self.raw) }
    }

    /// Pointer to the flat token array indexed by `ASTNode::tok_idx`.
    fn tokens(&self) -> *const msf_sys::Token {
        // SAFETY: `self.raw` is live; the token array is owned by it.
        unsafe { msf_sys::msf_tokens(self.raw) }
    }
}

impl Drop for Analysis {
    fn drop(&mut self) {
        // SAFETY: `self.raw` was returned by `msf_analyze` and is freed exactly
        // once, here. No `Node` can outlive `self` (they borrow it), so there are
        // no dangling readers.
        unsafe { msf_sys::msf_result_free(self.raw) };
    }
}

/// A borrowed view of one AST node. Tied to its [`Analysis`] by lifetime `'a`,
/// so it can never outlive the arena that backs it.
#[derive(Clone, Copy)]
pub struct Node<'a> {
    ptr: *const msf_sys::ASTNode,
    analysis: &'a Analysis,
}

impl<'a> Node<'a> {
    /// The kind of syntax this node represents.
    pub fn kind(&self) -> NodeKind {
        // SAFETY: `ptr` is a live, arena-owned node (non-null for `root()`'s
        // subtree). The `kind` field is a plain integer.
        let raw = unsafe { (*self.ptr).kind };
        NodeKind::from_raw(raw)
    }

    /// Iterator over this node's direct children, in source order.
    pub fn children(&self) -> Children<'a> {
        // SAFETY: reading the `first_child` link of a live node.
        let first = unsafe { (*self.ptr).first_child };
        Children {
            ptr: first,
            analysis: self.analysis,
        }
    }

    /// The integer value of an `IntegerLiteral` node, else `None`.
    pub fn int(&self) -> Option<i64> {
        if self.kind() == NodeKind::IntegerLiteral {
            // SAFETY: the `integer` union arm is active iff kind is
            // INTEGER_LITERAL, which we just checked.
            Some(unsafe { (*self.ptr).data.integer.ival })
        } else {
            None
        }
    }

    /// The value of a `BoolLiteral` node, else `None`.
    pub fn bool(&self) -> Option<bool> {
        if self.kind() == NodeKind::BoolLiteral {
            // SAFETY: the `boolean` arm is active iff kind is BOOL_LITERAL.
            Some(unsafe { (*self.ptr).data.boolean.bval } != 0)
        } else {
            None
        }
    }

    /// The value of a `FloatLiteral` node, else `None`.
    pub fn float(&self) -> Option<f64> {
        if self.kind() == NodeKind::FloatLiteral {
            // SAFETY: the `flt` arm is active iff kind is FLOAT_LITERAL.
            Some(unsafe { (*self.ptr).data.flt.fval })
        } else {
            None
        }
    }

    /// The source text of this node's first token (e.g. the name of an
    /// identifier-reference node). Owned copy — msf's `token_text` returns a
    /// thread-local buffer that the next call overwrites.
    pub fn text(&self) -> Option<String> {
        let src = self.analysis.source();
        let tokens = self.analysis.tokens();
        if src.is_null() || tokens.is_null() {
            return None;
        }
        // SAFETY: `tok_idx` is the node's starting token index into the
        // result-owned token array; `token_text` reads `tok` against `src` and
        // returns a NUL-terminated thread-local string, copied immediately.
        unsafe {
            let idx = (*self.ptr).tok_idx as usize;
            let tok = tokens.add(idx);
            let c = msf_sys::token_text(src, tok);
            if c.is_null() {
                None
            } else {
                Some(cstr_to_string(c))
            }
        }
    }
}

/// Iterator over a node's children produced by [`Node::children`].
pub struct Children<'a> {
    ptr: *const msf_sys::ASTNode,
    analysis: &'a Analysis,
}

impl<'a> Iterator for Children<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Node<'a>> {
        if self.ptr.is_null() {
            return None;
        }
        let current = self.ptr;
        // SAFETY: `current` is a live, arena-owned node; advancing along the
        // `next_sibling` link stays within the same arena.
        self.ptr = unsafe { (*current).next_sibling };
        Some(Node {
            ptr: current,
            analysis: self.analysis,
        })
    }
}

/// One analysis diagnostic (syntax or semantic error).
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub message: String,
    pub line: u32,
    pub col: u32,
}

/// Why [`Analysis::analyze`] could not produce a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalyzeError {
    /// `source` or `filename` contained an interior NUL byte.
    InteriorNul,
    /// msf reported an allocation failure (returned NULL).
    Allocation,
}

impl std::fmt::Display for AnalyzeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalyzeError::InteriorNul => write!(f, "source contained an interior NUL byte"),
            AnalyzeError::Allocation => write!(f, "msf reported an allocation failure"),
        }
    }
}

impl std::error::Error for AnalyzeError {}

/// Copy a borrowed C string into an owned `String` (lossy for non-UTF-8).
///
/// # Safety
/// `c` must be a valid, NUL-terminated C string pointer.
unsafe fn cstr_to_string(c: *const std::os::raw::c_char) -> String {
    if c.is_null() {
        return String::new();
    }
    CStr::from_ptr(c).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prove the FFI round-trips: analyze `print(42)`, walk the AST through the
    /// safe wrapper, and read the identifier and integer payloads.
    #[test]
    fn walks_print_42() {
        let a = Analysis::analyze("print(42)\n", "main.swift").unwrap();
        assert!(a.is_ok(), "unexpected diagnostics: {:?}", a.diagnostics());

        let root = a.root();
        assert_eq!(root.kind(), NodeKind::SourceFile);

        // source_file > expr_stmt > call_expr
        let stmt = root.children().next().expect("a statement");
        assert_eq!(stmt.kind(), NodeKind::ExprStmt);
        let call = stmt.children().next().expect("a call");
        assert_eq!(call.kind(), NodeKind::CallExpr);

        // call_expr children: callee identifier, then the integer argument.
        let mut kids = call.children();
        let callee = kids.next().expect("callee");
        assert_eq!(callee.kind(), NodeKind::IdentExpr);
        assert_eq!(callee.text().as_deref(), Some("print"));

        let arg = kids.next().expect("argument");
        assert_eq!(arg.kind(), NodeKind::IntegerLiteral);
        assert_eq!(arg.int(), Some(42));
    }

    /// A syntax error surfaces as a diagnostic, not a crash.
    #[test]
    fn reports_diagnostics() {
        let a = Analysis::analyze("let = =\n", "bad.swift").unwrap();
        assert!(!a.is_ok());
        assert!(!a.diagnostics().is_empty());
    }
}
