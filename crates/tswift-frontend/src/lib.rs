//! The tswift frontend: a pure-Rust Swift frontend.
//!
//! This crate is the **only** runtime-facing seam onto the Swift frontend. It
//! drives the pure-Rust pipeline (`tswift-lexer` → `tswift-parser` → `tswift-sema`)
//! and lowers the result, through the compatibility lowerer in [`compat`], into
//! the stable runtime-facing AST contract the runtime (`tswift-core` /
//! `-std`) consumes: [`Analysis`], [`Node`], and [`NodeKind`].
//!
//! There is no C dependency and no `unsafe`: the AST lives in an owned arena
//! ([`compat::RuntimeAst`]); a [`Node`] is a cheap cursor borrowing the
//! [`Analysis`] so it can never dangle.

#![forbid(unsafe_code)]

mod compat;
mod kind;
pub use kind::NodeKind;

/// An owned Swift analysis result: the typed, runtime-facing AST plus
/// diagnostics for one Swift source file.
pub struct Analysis {
    rust: compat::RuntimeAst,
}

impl Analysis {
    /// Tokenize, parse, and type-resolve `source`. `filename` is currently only
    /// part of the public signature for source compatibility; diagnostics carry
    /// line/column locations. Returns `Err` only if `source`/`filename` contain
    /// an interior NUL byte.
    pub fn analyze(source: &str, filename: &str) -> Result<Analysis, AnalyzeError> {
        if source.as_bytes().contains(&0) || filename.as_bytes().contains(&0) {
            return Err(AnalyzeError::InteriorNul);
        }
        Ok(Analysis {
            rust: compat::RuntimeAst::analyze(source),
        })
    }

    /// The root `source_file` node of the AST.
    pub fn root(&self) -> Node<'_> {
        Node {
            rust: self.rust.root(),
            analysis: self,
        }
    }

    /// Semantic/syntactic errors produced during analysis, in source order.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.rust.diagnostics()
    }

    /// Returns `true` if analysis produced no errors.
    pub fn is_ok(&self) -> bool {
        self.rust.is_ok()
    }
}

/// A borrowed view of one AST node. Tied to its [`Analysis`] by lifetime `'a`,
/// so it can never outlive the arena that backs it.
#[derive(Clone, Copy)]
pub struct Node<'a> {
    rust: compat::NodeId,
    analysis: &'a Analysis,
}

impl<'a> Node<'a> {
    fn cursor(&self, id: compat::NodeId) -> Node<'a> {
        Node {
            rust: id,
            analysis: self.analysis,
        }
    }

    /// The kind of syntax this node represents.
    pub fn kind(&self) -> NodeKind {
        self.analysis.rust.kind(self.rust)
    }

    /// Iterator over this node's direct children, in source order.
    pub fn children(&self) -> Children<'a> {
        Children {
            inner: self.analysis.rust.children(self.rust),
            analysis: self.analysis,
        }
    }

    /// The source text of this node's primary token (identifier name, literal
    /// text, operator), if any.
    pub fn text(&self) -> Option<String> {
        self.analysis.rust.text(self.rust)
    }

    /// For a [`NodeKind::BinaryExpr`]/`AssignExpr`/`CastExpr`/`PatternEnum`, the
    /// operator's (or case's) text.
    pub fn op_text(&self) -> Option<String> {
        self.analysis.rust.text(self.rust)
    }

    /// For a declaration node (var/let/func/param), its name.
    pub fn decl_name(&self) -> Option<String> {
        // `let`/`var` carry their name in a binding-pattern child; every other
        // declaration (func/struct/enum/…) carries it as the node's own text.
        for child in self.children() {
            match child.kind() {
                NodeKind::PatternValueBinding => return child.text(),
                NodeKind::PatternWildcard => return Some("_".to_string()),
                _ => {}
            }
        }
        self.analysis.rust.text(self.rust)
    }

    /// The integer value of an `IntegerLiteral` node, else `None`.
    pub fn int(&self) -> Option<i64> {
        self.analysis.rust.int(self.rust)
    }

    /// The value of a `BoolLiteral` node, else `None`.
    pub fn bool(&self) -> Option<bool> {
        self.analysis.rust.bool(self.rust)
    }

    /// The value of a `FloatLiteral` node, else `None`.
    pub fn float(&self) -> Option<f64> {
        self.analysis.rust.float(self.rust)
    }

    /// The 1-based source line of this node's first token.
    pub fn line(&self) -> u32 {
        self.analysis.rust.line(self.rust)
    }

    /// The resolved type name of this node (e.g. `Int`, `String`), if any.
    pub fn type_name(&self) -> Option<String> {
        self.analysis.rust.type_name(self.rust)
    }

    /// Whether this node carries the `async` effect modifier — `async let`
    /// on a binding, or `for await` on a loop.
    pub fn is_async(&self) -> bool {
        const MOD_ASYNC: u32 = 1 << 13;
        self.analysis.rust.modifiers(self.rust) & MOD_ASYNC != 0
    }

    /// For a `LetDecl`/`VarDecl`, whether it was written `async let`.
    pub fn is_async_let(&self) -> bool {
        self.is_async()
    }

    /// For a `break`/`continue` statement, its target loop label, if any.
    pub fn jump_label(&self) -> Option<String> {
        self.analysis.rust.text(self.rust)
    }

    /// For a `var`/`let` property, the ownership keyword (`weak`/`unowned`).
    pub fn ownership(&self) -> Option<String> {
        let names = self.modifier_names();
        if names.contains(&"weak") {
            Some("weak".into())
        } else if names.contains(&"unowned") {
            Some("unowned".into())
        } else {
            None
        }
    }

    /// For a `for`/`while`/`repeat` loop, its statement label, if any.
    pub fn loop_label(&self) -> Option<String> {
        self.analysis.rust.loop_label(self.rust)
    }

    /// For a `CaseClause`, whether it is `default` and its optional `where` guard.
    pub fn case_info(&self) -> CaseInfo<'a> {
        CaseInfo {
            is_default: self.analysis.rust.case_is_default(self.rust),
            where_expr: self
                .analysis
                .rust
                .case_where(self.rust)
                .map(|id| self.cursor(id)),
        }
    }

    /// The declaration modifier bitmask.
    pub fn modifiers(&self) -> u32 {
        self.analysis.rust.modifiers(self.rust)
    }

    /// The argument label of a call argument, if present.
    pub fn arg_label(&self) -> Option<String> {
        self.analysis.rust.arg_label(self.rust)
    }

    /// For an `AST_PARAM` node, its label/name/variadic/inout info.
    pub fn param_info(&self) -> ParamInfo {
        self.analysis.rust.param_info(self.rust)
    }

    /// For a `var`/`let`/`subscript`, its computed-accessor and observer bodies.
    pub fn var_accessors(&self) -> VarAccessors<'a> {
        let mut acc = VarAccessors {
            is_computed: false,
            has_setter: false,
            getter_body: None,
            setter_body: None,
            will_set_body: None,
            did_set_body: None,
            setter_param: None,
            will_set_param: None,
            did_set_param: None,
        };
        for child in self.children() {
            if child.kind() != NodeKind::AccessorDecl {
                continue;
            }
            let body = child.children().find(|c| c.kind() == NodeKind::Block);
            let param = child
                .children()
                .find(|c| c.kind() == NodeKind::Param)
                .and_then(|p| p.text());
            match child.text().as_deref() {
                Some("get") => {
                    acc.is_computed = true;
                    acc.getter_body = body;
                }
                Some("set") => {
                    acc.has_setter = true;
                    acc.setter_body = body;
                    acc.setter_param = param;
                }
                Some("willSet") => {
                    acc.will_set_body = body;
                    acc.will_set_param = param;
                }
                Some("didSet") => {
                    acc.did_set_body = body;
                    acc.did_set_param = param;
                }
                _ => {}
            }
        }
        acc
    }

    /// Decode this node's `modifiers` bitmask into a list of flag names.
    pub fn modifier_names(&self) -> Vec<&'static str> {
        let m = self.modifiers();
        const FLAGS: &[(u32, &str)] = &[
            (1 << 0, "public"),
            (1 << 1, "private"),
            (1 << 2, "internal"),
            (1 << 3, "fileprivate"),
            (1 << 4, "open"),
            (1 << 5, "static"),
            (1 << 6, "final"),
            (1 << 7, "override"),
            (1 << 8, "mutating"),
            (1 << 9, "nonmutating"),
            (1 << 10, "lazy"),
            (1 << 11, "weak"),
            (1 << 12, "unowned"),
            (1 << 13, "async"),
            (1 << 14, "throws"),
            (1 << 15, "rethrows"),
            (1 << 16, "indirect"),
            (1 << 17, "required"),
            (1 << 18, "convenience"),
            (1 << 19, "dynamic"),
            (1 << 26, "escaping"),
            (1 << 27, "autoclosure"),
            (1 << 28, "variadic"),
            (1 << 29, "failable"),
        ];
        FLAGS
            .iter()
            .filter(|(bit, _)| m & bit != 0)
            .map(|(_, name)| *name)
            .collect()
    }

    /// A recursive, human-readable dump of this subtree: kind, token text, line,
    /// resolved type, and decoded modifiers. This is the AST-inspection format
    /// behind `tswift dump`.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.dump_into(&mut out, 0);
        out
    }

    fn dump_into(&self, out: &mut String, depth: usize) {
        use std::fmt::Write as _;
        let indent = "  ".repeat(depth);
        let kind = self.kind();
        let raw = match kind {
            NodeKind::Other(n) => format!("Other({n})"),
            k => format!("{k:?}"),
        };
        let _ = write!(out, "{indent}{raw}");
        if let Some(text) = self.text() {
            if !text.is_empty() {
                let _ = write!(out, " {text:?}");
            }
        }
        let line = self.line();
        if line > 0 {
            let _ = write!(out, " L{line}");
        }
        if let Some(ty) = self.type_name() {
            let _ = write!(out, " :{ty}");
        }
        let mods = self.modifier_names();
        if !mods.is_empty() {
            let _ = write!(out, " [{}]", mods.join(","));
        }
        let _ = writeln!(out);
        for child in self.children() {
            child.dump_into(out, depth + 1);
        }
    }

    /// A structured JSON dump of this subtree (kind, text, line, type,
    /// modifiers, children), for tooling that wants to consume the AST shape.
    pub fn dump_json(&self) -> String {
        let mut out = String::new();
        self.dump_json_into(&mut out);
        out
    }

    fn dump_json_into(&self, out: &mut String) {
        use std::fmt::Write as _;
        let _ = write!(out, "{{\"kind\":\"{}\"", self.kind().name());
        if let NodeKind::Other(n) = self.kind() {
            let _ = write!(out, ",\"raw\":{n}");
        }
        if let Some(text) = self.text() {
            if !text.is_empty() {
                let _ = write!(out, ",\"text\":{}", json_string(&text));
            }
        }
        let line = self.line();
        if line > 0 {
            let _ = write!(out, ",\"line\":{line}");
        }
        if let Some(ty) = self.type_name() {
            let _ = write!(out, ",\"type\":{}", json_string(&ty));
        }
        let mods = self.modifier_names();
        if !mods.is_empty() {
            let parts: Vec<String> = mods.iter().map(|m| json_string(m)).collect();
            let _ = write!(out, ",\"modifiers\":[{}]", parts.join(","));
        }
        let mut children = self.children().peekable();
        if children.peek().is_some() {
            out.push_str(",\"children\":[");
            for (i, child) in children.enumerate() {
                if i > 0 {
                    out.push(',');
                }
                child.dump_json_into(out);
            }
            out.push(']');
        }
        out.push('}');
    }
}

/// Minimal JSON string escaping for [`Node::dump_json`].
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Iterator over a node's children produced by [`Node::children`].
pub struct Children<'a> {
    inner: compat::Children,
    analysis: &'a Analysis,
}

impl<'a> Iterator for Children<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Node<'a>> {
        self.inner.next().map(|id| Node {
            rust: id,
            analysis: self.analysis,
        })
    }
}

/// Decoded shape of a `switch` case clause.
#[derive(Clone, Copy)]
pub struct CaseInfo<'a> {
    /// `true` for the `default:` clause.
    pub is_default: bool,
    /// The `where` guard expression, if the clause has one.
    pub where_expr: Option<Node<'a>>,
}

/// Decoded shape of a function parameter.
#[derive(Debug, Clone)]
pub struct ParamInfo {
    /// External argument label used at call sites (`None` when written `_`).
    pub label: Option<String>,
    /// Internal name the parameter binds to inside the body.
    pub name: String,
    /// Whether the parameter is variadic (`T...`).
    pub variadic: bool,
    /// Whether the parameter is `@autoclosure` (its argument is deferred).
    pub autoclosure: bool,
    /// Whether the parameter is `inout`.
    pub is_inout: bool,
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
}

impl std::fmt::Display for AnalyzeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalyzeError::InteriorNul => write!(f, "source contained an interior NUL byte"),
        }
    }
}

impl std::error::Error for AnalyzeError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pipeline round-trips: analyze `print(42)`, walk the AST, read payloads.
    #[test]
    fn walks_print_42() {
        let a = Analysis::analyze("print(42)\n", "main.swift").unwrap();
        assert!(a.is_ok(), "unexpected diagnostics: {:?}", a.diagnostics());

        let root = a.root();
        assert_eq!(root.kind(), NodeKind::SourceFile);

        let stmt = root.children().next().expect("a statement");
        assert_eq!(stmt.kind(), NodeKind::ExprStmt);
        let call = stmt.children().next().expect("a call");
        assert_eq!(call.kind(), NodeKind::CallExpr);

        let mut kids = call.children();
        let callee = kids.next().expect("callee");
        assert_eq!(callee.kind(), NodeKind::IdentExpr);
        assert_eq!(callee.text().as_deref(), Some("print"));

        let arg = kids.next().expect("argument");
        assert_eq!(arg.kind(), NodeKind::IntegerLiteral);
        assert_eq!(arg.int(), Some(42));
    }

    /// `NodeKind` names its variants and `dump` reports kind/line/type/modifiers.
    #[test]
    fn dump_reports_modifiers_and_types() {
        let a = Analysis::analyze("struct S { static func f() throws {} }\n", "m.swift").unwrap();
        let func = a
            .root()
            .children()
            .next()
            .unwrap()
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .unwrap()
            .children()
            .find(|c| c.kind() == NodeKind::FuncDecl)
            .expect("func decl");
        let mods = func.modifier_names();
        assert!(mods.contains(&"static"), "mods: {mods:?}");
        assert_eq!(NodeKind::StructDecl.name(), "struct_decl");
    }

    /// A syntax error surfaces as a diagnostic, not a panic.
    #[test]
    fn reports_diagnostics() {
        let a = Analysis::analyze("let = =\n", "bad.swift").unwrap();
        assert!(!a.is_ok());
        assert!(!a.diagnostics().is_empty());
    }

    #[test]
    fn pins_minimal_runtime_dump_shape() {
        let a = Analysis::analyze("print(42)\n", "main.swift").unwrap();
        assert_eq!(
            a.root().dump(),
            "SourceFile L1\n  ExprStmt L1\n    CallExpr L1 :Void\n      IdentExpr \"print\" L1\n      IntegerLiteral \"42\" L1 :Int\n"
        );
    }

    /// Nominal declarations lower into the runtime-facing shape: name as text,
    /// inherited types as `Conformance` (with a `TypeIdent` child), members in a
    /// `Block`.
    #[test]
    fn lowers_struct_codable_shape() {
        let a = Analysis::analyze(
            "struct User: Codable {\n    let name: String\n    var age: Int\n}\n",
            "main.swift",
        )
        .unwrap();
        assert!(a.is_ok(), "unexpected diagnostics: {:?}", a.diagnostics());
        assert_eq!(
            a.root().dump(),
            "SourceFile L1\n  \
               StructDecl \"User\" L1\n    \
                 Conformance \"Codable\" L1\n      \
                   TypeIdent \"Codable\" L1\n    \
                 Block \"{\" L1\n      \
                   LetDecl L2 :String\n        \
                     PatternValueBinding \"name\" L2 :String\n        \
                     TypeIdent \"String\" L2\n      \
                   VarDecl L3 :Int\n        \
                     PatternValueBinding \"age\" L3 :Int\n        \
                     TypeIdent \"Int\" L3\n"
        );
    }

    /// Modifiers, attributes, ownership, and async-let metadata read through the
    /// existing helpers.
    #[test]
    fn lowers_modifiers_attributes_and_flags() {
        let a = Analysis::analyze(
            "@main\nstruct App {\n    @State var count = 0\n    static let shared = 1\n    lazy var cache = 2\n    weak var owner = 3\n    func main() {\n        async let job = run()\n        let plain = 1\n    }\n}\n",
            "main.swift",
        )
        .unwrap();
        let app = a
            .root()
            .children()
            .find(|c| c.kind() == NodeKind::StructDecl)
            .unwrap();
        assert!(app
            .children()
            .any(|c| c.kind() == NodeKind::Attribute && c.text().as_deref() == Some("main")));
        let body = app
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .unwrap();
        let member = |name: &str| {
            body.children()
                .find(|c| c.decl_name().as_deref() == Some(name))
                .unwrap_or_else(|| panic!("member {name}"))
        };
        assert!(member("count")
            .children()
            .any(|c| c.kind() == NodeKind::Attribute && c.text().as_deref() == Some("State")));
        assert!(member("shared").modifier_names().contains(&"static"));
        assert!(member("cache").modifier_names().contains(&"lazy"));
        assert_eq!(member("owner").ownership().as_deref(), Some("weak"));

        let func_body = body
            .children()
            .find(|c| c.kind() == NodeKind::FuncDecl)
            .unwrap()
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .unwrap();
        let lets: Vec<_> = func_body
            .children()
            .filter(|c| c.kind() == NodeKind::LetDecl)
            .collect();
        assert!(lets[0].is_async_let());
        assert!(!lets[1].is_async_let());
    }

    /// Patterns lower into the runtime-facing shapes: enum-case (`op_text` = case
    /// name), tuple, optional binding, `where` via `case_info`, and the nested
    /// enum-case declaration shape.
    #[test]
    fn lowers_patterns() {
        let a = Analysis::analyze(
            "enum E { case a(Int) }\nlet o: Int? = 1\nif let v = o { }\nswitch x {\ncase .a(let n) where n > 0: break\ncase (let p, _): break\ndefault: break\n}\n",
            "main.swift",
        )
        .unwrap();
        let root = a.root();
        let e = root
            .children()
            .find(|c| c.kind() == NodeKind::EnumDecl)
            .unwrap();
        let block = e.children().find(|c| c.kind() == NodeKind::Block).unwrap();
        let case = block.children().next().unwrap();
        assert_eq!(case.kind(), NodeKind::EnumCaseDecl);
        let element = case.children().next().unwrap();
        assert_eq!(element.kind(), NodeKind::EnumElementDecl);
        assert_eq!(element.text().as_deref(), Some("a"));

        let if_stmt = root
            .children()
            .find(|c| c.kind() == NodeKind::IfStmt)
            .unwrap();
        assert!(if_stmt
            .children()
            .any(|c| c.kind() == NodeKind::OptionalBinding));

        let sw = root
            .children()
            .find(|c| c.kind() == NodeKind::SwitchStmt)
            .unwrap();
        let clauses: Vec<_> = sw
            .children()
            .filter(|c| c.kind() == NodeKind::CaseClause)
            .collect();
        let enum_pat = clauses[0].children().next().unwrap();
        assert_eq!(enum_pat.kind(), NodeKind::PatternEnum);
        assert_eq!(enum_pat.op_text().as_deref(), Some("a"));
        assert!(clauses[0].case_info().where_expr.is_some());
        assert_eq!(
            clauses[1].children().next().unwrap().kind(),
            NodeKind::PatternTuple
        );
        assert!(clauses[2].case_info().is_default);
    }

    /// Effects, directives, and concurrency lower into their runtime wrappers.
    #[test]
    fn lowers_effects_directives_concurrency() {
        let a = Analysis::analyze(
            "#if DEBUG\nlet mode = 1\n#endif\nfunc f() {\n    let r = try? g()\n    let l = #line\n    let v = await t.value\n    for await x in s { use(x) }\n}\n",
            "main.swift",
        )
        .unwrap();
        let root = a.root();
        assert!(root
            .children()
            .any(|c| c.kind() == NodeKind::LetDecl && c.decl_name().as_deref() == Some("mode")));
        let body = root
            .children()
            .find(|c| c.kind() == NodeKind::FuncDecl)
            .unwrap()
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .unwrap();
        let stmts: Vec<_> = body.children().collect();
        let try_expr = stmts[0].children().last().unwrap();
        assert_eq!(try_expr.kind(), NodeKind::TryExpr);
        assert_eq!(try_expr.op_text().as_deref(), Some("?"));
        let line_macro = stmts[1].children().last().unwrap();
        assert_eq!(line_macro.kind(), NodeKind::MacroExpansion);
        assert_eq!(line_macro.text().as_deref(), Some("line"));
        assert_eq!(
            stmts[2].children().last().unwrap().kind(),
            NodeKind::AwaitExpr
        );
        let for_stmt = stmts[3];
        assert!(for_stmt.is_async());
        let binding = for_stmt
            .children()
            .find(|c| c.kind() == NodeKind::PatternValueBinding)
            .unwrap();
        assert_eq!(binding.text().as_deref(), Some("x"));
    }

    /// Calls, accessors, subscripts, and custom operators lower into the
    /// runtime-facing contract.
    #[test]
    fn lowers_calls_accessors_subscripts() {
        let a = Analysis::analyze(
            "struct Box {\n    var items: [Int]\n    var first: Int { return items[0] }\n    var n: Int = 0 { didSet { } }\n    subscript(i: Int) -> Int { return items[i] }\n    mutating func add(_ xs: Int..., at j: Int) { }\n}\nfn(label: 1, &place)\n",
            "main.swift",
        )
        .unwrap();
        let root = a.root();
        let body = root
            .children()
            .find(|c| c.kind() == NodeKind::StructDecl)
            .unwrap()
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .unwrap();
        let first = body
            .children()
            .find(|c| c.decl_name().as_deref() == Some("first"))
            .unwrap();
        assert!(first.var_accessors().is_computed);
        let n = body
            .children()
            .find(|c| c.decl_name().as_deref() == Some("n"))
            .unwrap();
        assert!(n.var_accessors().did_set_body.is_some());
        let sub = body
            .children()
            .find(|c| c.kind() == NodeKind::SubscriptDecl)
            .unwrap();
        assert!(sub.var_accessors().getter_body.is_some());
        let add = body
            .children()
            .find(|c| c.kind() == NodeKind::FuncDecl && c.text().as_deref() == Some("add"))
            .unwrap();
        assert!(add
            .children()
            .filter(|c| c.kind() == NodeKind::Param)
            .any(|p| p.param_info().variadic));

        let call = root
            .children()
            .find(|c| c.kind() == NodeKind::ExprStmt)
            .unwrap()
            .children()
            .next()
            .unwrap();
        let args: Vec<_> = call.children().skip(1).collect();
        assert_eq!(args[0].arg_label().as_deref(), Some("label"));
        assert_eq!(args[1].kind(), NodeKind::InoutExpr);
    }
}
