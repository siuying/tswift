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

    /// The `TokenType` discriminant of the token at flat index `idx`.
    fn token_type_at(&self, idx: u32) -> Option<u32> {
        let tokens = self.tokens();
        if tokens.is_null() {
            return None;
        }
        // SAFETY: `idx` indexes the result-owned token array.
        Some(unsafe { (*tokens.add(idx as usize)).type_ })
    }

    /// Whether the token at flat index `idx` had a newline before it.
    fn token_has_leading_newline_at(&self, idx: u32) -> bool {
        let tokens = self.tokens();
        if tokens.is_null() {
            return false;
        }
        // SAFETY: `idx` indexes the result-owned token array.
        unsafe { (*tokens.add(idx as usize)).has_leading_newline != 0 }
    }

    /// The source text of the token at flat index `idx`, owned copy.
    fn token_text_at(&self, idx: u32) -> Option<String> {
        let src = self.source();
        let tokens = self.tokens();
        if src.is_null() || tokens.is_null() {
            return None;
        }
        // SAFETY: `idx` indexes the result-owned token array; `token_text`
        // returns a thread-local NUL-terminated string we copy immediately.
        unsafe {
            let tok = tokens.add(idx as usize);
            let c = msf_sys::token_text(src, tok);
            if c.is_null() {
                None
            } else {
                Some(cstr_to_string(c))
            }
        }
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
    /// For a [`NodeKind::BinaryExpr`]/`AssignExpr`/`CastExpr`, the operator's
    /// source text via its `op_tok`. For other nodes, `None`.
    pub fn op_text(&self) -> Option<String> {
        // SAFETY: reading the `binary.op_tok` arm. msf populates this arm for
        // BINARY/ASSIGN/CAST nodes; for others the token index may be stale, so
        // callers should only use this on those kinds.
        let op_tok = unsafe { (*self.ptr).data.binary.op_tok };
        self.analysis.token_text_at(op_tok)
    }

    /// For a declaration node (var/let/func/param), its name via `name_tok`.
    pub fn decl_name(&self) -> Option<String> {
        // SAFETY: the `var.name_tok`/`func.name_tok` arms overlap in layout
        // (both first field u32 name_tok); valid for decl nodes.
        let name_tok = unsafe { (*self.ptr).data.var.name_tok };
        self.analysis.token_text_at(name_tok)
    }

    /// The first token index of this node (`ASTNode.tok_idx`).
    fn tok_idx(&self) -> u32 {
        // SAFETY: `tok_idx` is a plain integer field on a live node.
        unsafe { (*self.ptr).tok_idx }
    }

    /// For a `break`/`continue` statement, the target loop label that follows
    /// the keyword (e.g. `break outer`), if any. msf points the node's `tok_idx`
    /// at the label token itself when present, else at the following token.
    pub fn jump_label(&self) -> Option<String> {
        const TOK_IDENTIFIER: u32 = 1;
        let idx = self.tok_idx();
        if self.analysis.token_type_at(idx) == Some(TOK_IDENTIFIER)
            && !self.analysis.token_has_leading_newline_at(idx)
        {
            self.analysis.token_text_at(idx)
        } else {
            None
        }
    }

    /// For a `var`/`let` property, the ownership keyword (`weak`/`unowned`)
    /// written before its declaration keyword, if any. msf does not surface this
    /// in `modifiers`, so we recover it from the token stream.
    pub fn ownership(&self) -> Option<String> {
        let ti = self.tok_idx();
        if ti < 2 {
            return None;
        }
        let kw = self.analysis.token_text_at(ti - 1)?;
        if !matches!(kw.as_str(), "var" | "let") {
            return None;
        }
        match self.analysis.token_text_at(ti - 2).as_deref() {
            Some("weak") => Some("weak".into()),
            Some("unowned") => Some("unowned".into()),
            _ => None,
        }
    }

    /// For a `for`/`while`/`repeat` loop, the statement label written before the
    /// loop keyword (e.g. `outer: for …`), if any.
    pub fn loop_label(&self) -> Option<String> {
        const TOK_IDENTIFIER: u32 = 1;
        let ti = self.tok_idx();
        if ti < 3 {
            return None;
        }
        let kw = self.analysis.token_text_at(ti - 1)?;
        if !matches!(kw.as_str(), "for" | "while" | "repeat") {
            return None;
        }
        if self.analysis.token_text_at(ti - 2).as_deref() != Some(":") {
            return None;
        }
        if self.analysis.token_type_at(ti - 3) == Some(TOK_IDENTIFIER) {
            self.analysis.token_text_at(ti - 3)
        } else {
            None
        }
    }

    /// For an `AST_CASE_CLAUSE`, whether it is the `default` clause, and its
    /// optional `where` guard expression.
    pub fn case_info(&self) -> CaseInfo<'a> {
        // SAFETY: the `cas` arm is active for CASE_CLAUSE nodes.
        let (is_default, has_guard, where_ptr) = unsafe {
            let c = (*self.ptr).data.cas;
            (c.is_default != 0, c.has_guard != 0, c.where_expr)
        };
        let where_expr = if has_guard && !where_ptr.is_null() {
            Some(Node {
                ptr: where_ptr,
                analysis: self.analysis,
            })
        } else {
            None
        };
        CaseInfo {
            is_default,
            where_expr,
        }
    }

    /// The declaration modifier bitmask (`ASTNode.modifiers`).
    pub fn modifiers(&self) -> u32 {
        // SAFETY: `modifiers` is a plain integer field on a live node.
        unsafe { (*self.ptr).modifiers }
    }

    /// The argument label of a call argument (`arg_label_tok`), if present.
    pub fn arg_label(&self) -> Option<String> {
        // SAFETY: `arg_label_tok` is a plain integer field; 0 means none.
        let tok = unsafe { (*self.ptr).arg_label_tok };
        if tok == 0 {
            None
        } else {
            self.analysis.token_text_at(tok)
        }
    }

    /// For an `AST_PARAM` node, its external argument label (`None` for `_`),
    /// internal binding name, and whether it is variadic (`T...`).
    pub fn param_info(&self) -> ParamInfo {
        // SAFETY: reading the plain `tok_idx`/`modifiers` integer fields.
        let first_tok = unsafe { (*self.ptr).tok_idx };
        let modifiers = unsafe { (*self.ptr).modifiers };
        let first = self.analysis.token_text_at(first_tok).unwrap_or_default();
        let second = self
            .analysis
            .token_text_at(first_tok + 1)
            .unwrap_or_default();
        // `extName intName: Type` vs single `name: Type`.
        let (label, name) = if second == ":" || second.is_empty() {
            let label = if first == "_" {
                None
            } else {
                Some(first.clone())
            };
            (label, first)
        } else {
            let label = if first == "_" {
                None
            } else {
                Some(first.clone())
            };
            (label, second)
        };
        const MOD_VARIADIC: u32 = 1 << 28;
        let is_inout = self
            .children()
            .any(|c| c.kind() == crate::NodeKind::TypeInout);
        ParamInfo {
            label,
            name,
            variadic: modifiers & MOD_VARIADIC != 0,
            is_inout,
        }
    }

    /// For a `var`/`let` property, its computed-accessor bodies and observer
    /// bodies (with their parameter names), read from msf's `var` union arm.
    pub fn var_accessors(&self) -> VarAccessors<'a> {
        // SAFETY: the `var` arm is active for VAR_DECL/LET_DECL nodes.
        let v = unsafe { (*self.ptr).data.var };
        let node = |p: *mut msf_sys::ASTNode| {
            if p.is_null() {
                None
            } else {
                Some(Node {
                    ptr: p,
                    analysis: self.analysis,
                })
            }
        };
        VarAccessors {
            is_computed: v.is_computed != 0,
            has_setter: v.has_setter != 0,
            getter_body: node(v.getter_body),
            setter_body: node(v.setter_body),
            will_set_body: node(v.will_set_body),
            did_set_body: node(v.did_set_body),
            setter_param: opt_tok(self.analysis, v.setter_param_name_tok),
            will_set_param: opt_tok(self.analysis, v.will_set_param_name_tok),
            did_set_param: opt_tok(self.analysis, v.did_set_param_name_tok),
        }
    }

    /// The resolved type name of this node (e.g. `Int`, `UInt8`, `Double`,
    /// `[String]`, `Int?`), as produced by msf's `type_to_string`. `None` if
    /// the node has no resolved type.
    pub fn type_name(&self) -> Option<String> {
        // SAFETY: `type_` is a result-owned `TypeInfo*` or NULL.
        let ty = unsafe { (*self.ptr).type_ };
        if ty.is_null() {
            return None;
        }
        let mut buf = [0i8; 128];
        // SAFETY: `ty` is non-null and result-owned; `buf` is a valid writable
        // buffer of `len` bytes; `type_to_string` NUL-terminates within it.
        unsafe {
            let p = msf_sys::type_to_string(ty, buf.as_mut_ptr(), buf.len());
            if p.is_null() {
                None
            } else {
                Some(cstr_to_string(p))
            }
        }
    }

    /// A recursive debug dump of this subtree (kind + token text), for tests.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.dump_into(&mut out, 0);
        out
    }

    fn dump_into(&self, out: &mut String, depth: usize) {
        use std::fmt::Write as _;
        let indent = "  ".repeat(depth);
        let text = self.text().unwrap_or_default();
        let _ = writeln!(out, "{indent}{:?} {:?}", self.kind(), text);
        for child in self.children() {
            child.dump_into(out, depth + 1);
        }
    }
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

/// Decoded shape of a `switch` case clause (`AST_CASE_CLAUSE`).
#[derive(Clone, Copy)]
pub struct CaseInfo<'a> {
    /// `true` for the `default:` clause.
    pub is_default: bool,
    /// The `where` guard expression, if the clause has one.
    pub where_expr: Option<Node<'a>>,
}

/// Decoded shape of a function parameter (`AST_PARAM`).
#[derive(Debug, Clone)]
pub struct ParamInfo {
    /// External argument label used at call sites (`None` when written `_`).
    pub label: Option<String>,
    /// Internal name the parameter binds to inside the body.
    pub name: String,
    /// Whether the parameter is variadic (`T...`).
    pub variadic: bool,
    /// Whether the parameter is `inout`.
    pub is_inout: bool,
}

/// Read the text of token `tok`, treating index 0 (msf's "no token") as `None`.
fn opt_tok(analysis: &Analysis, tok: u32) -> Option<String> {
    if tok == 0 {
        None
    } else {
        analysis.token_text_at(tok)
    }
}

/// Computed-accessor and observer bodies of a `var`/`let` property.
#[derive(Clone)]
pub struct VarAccessors<'a> {
    pub is_computed: bool,
    pub has_setter: bool,
    pub getter_body: Option<Node<'a>>,
    pub setter_body: Option<Node<'a>>,
    pub will_set_body: Option<Node<'a>>,
    pub did_set_body: Option<Node<'a>>,
    pub setter_param: Option<String>,
    pub will_set_param: Option<String>,
    pub did_set_param: Option<String>,
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
