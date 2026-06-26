//! Rust-backend compatibility tree for the runtime-facing frontend facade.
//!
//! This module is intentionally internal: `qswift-core` keeps consuming
//! `Analysis`/`Node`/`NodeKind`, while the Rust parser/sema pipeline lowers into
//! the historical runtime-facing node vocabulary here.

use crate::{Diagnostic, NodeKind, ParamInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodeId(usize);

#[derive(Debug, Clone)]
pub(crate) struct RuntimeAst {
    nodes: Vec<RuntimeNode>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
struct RuntimeNode {
    kind: NodeKind,
    text: Option<String>,
    line: u32,
    ty: Option<String>,
    modifier_bits: u32,
    arg_label: Option<String>,
    /// For a `CaseClause`, whether it is the `default:` clause.
    is_default: bool,
    /// For a `CaseClause`, its `where` guard expression, if any.
    where_expr: Option<NodeId>,
    /// For a `for await` loop, the real loop binding (the node's text is the
    /// `await` sentinel the runtime keys on; `token_text_offset(1)` returns it).
    for_await_binding: Option<String>,
    /// For a `for`/`while`/`repeat` loop, its statement label (`outer:`), if any.
    loop_label: Option<String>,
    children: Vec<NodeId>,
}

pub(crate) struct Children {
    ids: Vec<NodeId>,
    pos: usize,
}

impl RuntimeAst {
    pub(crate) fn analyze(source: &str) -> RuntimeAst {
        let mut ast = match qswift_parser::parse(source) {
            Ok(ast) => ast,
            Err(e) => return RuntimeAst::diagnostic(e.message, e.line, e.col),
        };
        let diagnostics = qswift_sema::resolve(&mut ast)
            .into_iter()
            .map(|d| Diagnostic {
                message: d.message,
                line: d.line,
                col: d.col,
            })
            .collect();

        let mut out = RuntimeAst {
            nodes: Vec::new(),
            diagnostics,
        };
        out.lower_node(ast.node(ast.root()));
        out
    }

    fn diagnostic(message: String, line: u32, col: u32) -> RuntimeAst {
        RuntimeAst {
            nodes: vec![RuntimeNode {
                kind: NodeKind::SourceFile,
                text: None,
                line: 1,
                ty: None,
                modifier_bits: 0,
                arg_label: None,
                is_default: false,
                where_expr: None,
                for_await_binding: None,
                loop_label: None,
                children: Vec::new(),
            }],
            diagnostics: vec![Diagnostic { message, line, col }],
        }
    }

    /// Allocate a runtime node and return its id.
    fn alloc(&mut self, kind: NodeKind, text: Option<String>, line: u32) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(RuntimeNode {
            kind,
            text,
            line,
            ty: None,
            modifier_bits: 0,
            arg_label: None,
            is_default: false,
            where_expr: None,
            for_await_binding: None,
            loop_label: None,
            children: Vec::new(),
        });
        id
    }

    fn set_children(&mut self, id: NodeId, children: Vec<NodeId>) {
        self.nodes[id.0].children = children;
    }

    fn lower_node(&mut self, node: qswift_ast::Node<'_>) -> NodeId {
        use qswift_ast::NodeKind as K;
        match node.kind() {
            K::StructDecl
            | K::EnumDecl
            | K::ClassDecl
            | K::ActorDecl
            | K::ProtocolDecl
            | K::ExtensionDecl => return self.lower_nominal(node),
            K::LetDecl | K::VarDecl => return self.lower_binding(node),
            K::ForStmt => return self.lower_for(node),
            K::CaseClause => return self.lower_case_clause(node),
            K::EnumCaseDecl => return self.lower_enum_case(node),
            K::IfStmt | K::GuardStmt | K::WhileStmt => return self.lower_conditional(node),
            _ => {}
        }

        let kind = map_kind(node.kind());
        // Source-location / message directives (`#line`, `#file`, `#warning`,
        // …) carry their name without the leading `#`, matching the runtime's
        // `eval_macro` keys.
        let text = match node.kind() {
            qswift_ast::NodeKind::CompilerDirective => {
                node.text().map(|t| t.trim_start_matches('#').to_string())
            }
            _ => node.text().map(ToOwned::to_owned),
        };
        let id = self.alloc(kind, text, node.line());
        self.nodes[id.0].ty = node.type_name().map(ToOwned::to_owned);
        self.nodes[id.0].modifier_bits = modifier_bits(node.modifiers());
        // The runtime reads the optional-cast flag (`as?`) from bit 0x800 on a
        // CastExpr, not from its operator text.
        if node.kind() == qswift_ast::NodeKind::CastExpr && node.text() == Some("as?") {
            self.nodes[id.0].modifier_bits |= 0x800;
        }
        self.nodes[id.0].arg_label = node.arg_label().map(ToOwned::to_owned);
        // A `repeat` loop's text is its statement label, if any.
        if node.kind() == qswift_ast::NodeKind::RepeatStmt {
            self.nodes[id.0].loop_label = node.text().map(ToOwned::to_owned);
        }

        let children = self.lower_child_list(node.children());
        self.set_children(id, children);
        id
    }

    /// Lower a child sequence, splicing resolved `#if` conditional-compilation
    /// directives inline (as msf does) so their active-branch statements land in
    /// the enclosing scope instead of behind a wrapper node the runtime would
    /// skip.
    fn lower_child_list<'b>(
        &mut self,
        children: impl Iterator<Item = qswift_ast::Node<'b>>,
    ) -> Vec<NodeId> {
        let mut out = Vec::new();
        for child in children {
            if child.kind() == qswift_ast::NodeKind::CompilerDirective && child.text() == Some("#if")
            {
                out.extend(self.lower_child_list(child.children()));
            } else {
                out.push(self.lower_node(child));
            }
        }
        out
    }

    /// Lower a nominal declaration (struct/enum/class/protocol/extension) into
    /// the runtime-facing shape: name as text, inherited types as `Conformance`
    /// children, attributes as `Attribute` children, and members wrapped in a
    /// `Block`. This is the shape `quick-swift-core`'s `register_*` expects.
    fn lower_nominal(&mut self, node: qswift_ast::Node<'_>) -> NodeId {
        use qswift_ast::NodeKind as K;
        let kind = map_kind(node.kind());
        let id = self.alloc(kind, node.text().map(ToOwned::to_owned), node.line());
        self.nodes[id.0].ty = node.type_name().map(ToOwned::to_owned);
        self.nodes[id.0].modifier_bits = modifier_bits(node.modifiers());

        let mut children: Vec<NodeId> = Vec::new();
        let mut members: Vec<NodeId> = Vec::new();
        let line = node.line();
        for child in node.children() {
            match child.kind() {
                // Attributes (`@main`, …) stay as direct children of the decl.
                K::Attribute => children.push(self.lower_node(child)),
                // Generic parameters stay as direct children.
                K::GenericParam => children.push(self.lower_node(child)),
                // Inherited protocols / superclass / raw type lower into
                // `Conformance` nodes the runtime reads via `record_conformances`.
                K::TypeRef => {
                    let name = child.text().map(ToOwned::to_owned);
                    let conf = self.alloc(NodeKind::Conformance, name.clone(), child.line());
                    let ident = self.alloc(NodeKind::TypeIdent, name, child.line());
                    self.set_children(conf, vec![ident]);
                    children.push(conf);
                }
                // Everything else is a member of the type body.
                _ => members.push(self.lower_node(child)),
            }
        }
        let block = self.alloc(NodeKind::Block, Some("{".to_string()), line);
        self.set_children(block, members);
        children.push(block);
        self.set_children(id, children);
        id
    }

    /// Lower a `switch` case clause into the runtime-facing shape: pattern
    /// children followed by the body `Block`, with the `where` guard and the
    /// `default` marker exposed separately through `case_info` (not as a
    /// pattern child the runtime would try to match).
    fn lower_case_clause(&mut self, node: qswift_ast::Node<'_>) -> NodeId {
        use qswift_ast::NodeKind as K;
        let text = node.text().map(ToOwned::to_owned);
        let is_default = text.as_deref() == Some("default");
        let id = self.alloc(NodeKind::CaseClause, text, node.line());
        let mut children: Vec<NodeId> = Vec::new();
        let mut where_expr: Option<NodeId> = None;
        for child in node.children() {
            if child.kind() == K::WhereClause {
                // The guard's single child is the condition expression.
                where_expr = child.children().next().map(|c| self.lower_node(c));
            } else {
                children.push(self.lower_node(child));
            }
        }
        self.nodes[id.0].is_default = is_default;
        self.nodes[id.0].where_expr = where_expr;
        self.set_children(id, children);
        id
    }

    /// Lower an enum `case` into the runtime-facing nesting the interpreter's
    /// `register_enum` walks: `EnumCaseDecl("case") > EnumElementDecl(name)`,
    /// where associated-value types become `Param > TypeIdent` children and a
    /// raw value stays as a plain value child.
    fn lower_enum_case(&mut self, node: qswift_ast::Node<'_>) -> NodeId {
        use qswift_ast::NodeKind as K;
        let outer = self.alloc(
            NodeKind::EnumCaseDecl,
            Some("case".to_string()),
            node.line(),
        );
        let element = self.alloc(
            NodeKind::EnumElementDecl,
            node.text().map(ToOwned::to_owned),
            node.line(),
        );
        let mut element_children: Vec<NodeId> = Vec::new();
        for child in node.children() {
            if child.kind() == K::TypeRef {
                // An associated value: wrap its type in a `Param`.
                let ident = self.lower_node(child); // -> TypeIdent
                let param = self.alloc(NodeKind::Param, None, child.line());
                self.set_children(param, vec![ident]);
                element_children.push(param);
            } else {
                // A raw value expression (`case a = 1`).
                element_children.push(self.lower_node(child));
            }
        }
        self.set_children(element, element_children);
        self.set_children(outer, vec![element]);
        outer
    }

    /// Lower an `if`/`guard`/`while` statement, converting any `let`/`var`
    /// condition binding into an `OptionalBinding` node (text = name, single
    /// child = the unwrapped expression) as the runtime's `eval_cond_list`
    /// expects. Boolean conditions and the body blocks lower normally.
    fn lower_conditional(&mut self, node: qswift_ast::Node<'_>) -> NodeId {
        use qswift_ast::NodeKind as K;
        let kind = map_kind(node.kind());
        let id = self.alloc(kind, node.text().map(ToOwned::to_owned), node.line());
        self.nodes[id.0].ty = node.type_name().map(ToOwned::to_owned);
        // A `while` loop's text is its statement label, if any.
        if node.kind() == qswift_ast::NodeKind::WhileStmt {
            self.nodes[id.0].loop_label = node.text().map(ToOwned::to_owned);
        }
        let mut children: Vec<NodeId> = Vec::new();
        for child in node.children() {
            match child.kind() {
                K::LetDecl | K::VarDecl => {
                    children.push(self.lower_optional_binding(child));
                }
                _ => children.push(self.lower_node(child)),
            }
        }
        self.set_children(id, children);
        id
    }

    /// Lower a conditional binding. A simple `let name = expr` (the binding
    /// pattern is a bare `NamePattern`) becomes an `OptionalBinding`. A refutable
    /// `case` pattern (`if case .x(let v) = expr`, a tuple/enum/range pattern)
    /// becomes a `CaseCondition` carrying the lowered pattern and the matched
    /// expression, which the runtime evaluates by pattern match.
    fn lower_optional_binding(&mut self, node: qswift_ast::Node<'_>) -> NodeId {
        use qswift_ast::NodeKind as K;
        let mut name: Option<String> = None;
        let mut binding_pattern: Option<qswift_ast::Node<'_>> = None;
        let mut init: Option<NodeId> = None;
        for child in node.children() {
            match child.kind() {
                K::NamePattern if name.is_none() && binding_pattern.is_none() => {
                    name = child.text().map(ToOwned::to_owned);
                }
                K::EnumCasePattern | K::TuplePattern | K::RangePattern if binding_pattern.is_none() => {
                    binding_pattern = Some(child);
                }
                K::TypeRef => {}
                _ => init = Some(self.lower_node(child)),
            }
        }
        // A refutable `case` pattern condition.
        if let Some(pattern) = binding_pattern {
            let pattern_id = self.lower_node(pattern);
            let id = self.alloc(NodeKind::CaseCondition, None, node.line());
            let mut children = vec![pattern_id];
            if let Some(init) = init {
                children.push(init);
            }
            self.set_children(id, children);
            return id;
        }
        let id = self.alloc(NodeKind::OptionalBinding, name, node.line());
        if let Some(init) = init {
            self.set_children(id, vec![init]);
        }
        id
    }

    /// Lower a `for pattern in seq { body }` loop into the runtime-facing shape:
    /// the loop variable name becomes the `ForStmt`'s text (as msf anchors it),
    /// and the simple binding pattern is not re-emitted as a child — so the
    /// runtime reads the iterable as the first non-`Block` child.
    fn lower_for(&mut self, node: qswift_ast::Node<'_>) -> NodeId {
        use qswift_ast::NodeKind as K;
        let id = self.alloc(NodeKind::ForStmt, None, node.line());
        // The `for` node's text is its statement label (`outer:`), if any; the
        // loop binding comes from the pattern child below.
        self.nodes[id.0].loop_label = node.text().map(ToOwned::to_owned);
        // `for await` is recorded as the async modifier by the parser; the
        // runtime detects it via the `await` sentinel text (ADR-0005), reading
        // the real binding through `token_text_offset(1)`.
        const MOD_ASYNC: u32 = 1 << 13;
        let is_await = modifier_bits(node.modifiers()) & MOD_ASYNC != 0;
        let mut binding: Option<String> = None;
        let mut children: Vec<NodeId> = Vec::new();
        for child in node.children() {
            match child.kind() {
                K::NamePattern if binding.is_none() => {
                    binding = child.text().map(ToOwned::to_owned);
                }
                K::WildcardPattern if binding.is_none() => {
                    binding = Some("_".to_string());
                }
                _ => children.push(self.lower_node(child)),
            }
        }
        if is_await {
            self.nodes[id.0].for_await_binding = binding;
            self.nodes[id.0].text = Some("await".to_string());
        } else {
            self.nodes[id.0].text = binding;
        }
        self.set_children(id, children);
        id
    }

    /// Lower a `let`/`var` binding into the runtime-facing shape: the binding
    /// name as the node's text, a `TypeIdent` child for the annotation, then the
    /// initializer/accessor children. The runtime reads the name via
    /// `decl_name()` and the default via the first value child.
    fn lower_binding(&mut self, node: qswift_ast::Node<'_>) -> NodeId {
        use qswift_ast::NodeKind as K;
        let kind = map_kind(node.kind());
        let id = self.alloc(kind, None, node.line());
        self.nodes[id.0].ty = node.type_name().map(ToOwned::to_owned);
        self.nodes[id.0].modifier_bits = modifier_bits(node.modifiers());

        let mut name: Option<String> = None;
        let mut children: Vec<NodeId> = Vec::new();
        for child in node.children() {
            match child.kind() {
                // The simple name pattern becomes the decl's own text; we do
                // not keep it as a child (matching the msf `var x` shape).
                K::NamePattern if name.is_none() => {
                    name = child.text().map(ToOwned::to_owned);
                }
                // A wildcard binding (`let _ = e`) names itself `_`.
                K::WildcardPattern if name.is_none() => {
                    name = Some("_".to_string());
                }
                _ => children.push(self.lower_node(child)),
            }
        }
        self.nodes[id.0].text = name;
        self.set_children(id, children);
        id
    }

    pub(crate) fn root(&self) -> NodeId {
        NodeId(0)
    }

    pub(crate) fn diagnostics(&self) -> Vec<Diagnostic> {
        self.diagnostics.clone()
    }

    pub(crate) fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub(crate) fn kind(&self, id: NodeId) -> NodeKind {
        self.node(id).kind
    }

    pub(crate) fn text(&self, id: NodeId) -> Option<String> {
        self.node(id).text.clone()
    }

    pub(crate) fn line(&self, id: NodeId) -> u32 {
        self.node(id).line
    }

    pub(crate) fn type_name(&self, id: NodeId) -> Option<String> {
        self.node(id).ty.clone()
    }

    pub(crate) fn modifiers(&self, id: NodeId) -> u32 {
        self.node(id).modifier_bits
    }

    pub(crate) fn arg_label(&self, id: NodeId) -> Option<String> {
        self.node(id).arg_label.clone()
    }

    pub(crate) fn case_is_default(&self, id: NodeId) -> bool {
        self.node(id).is_default
    }

    pub(crate) fn case_where(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).where_expr
    }

    pub(crate) fn for_await_binding(&self, id: NodeId) -> Option<String> {
        self.node(id).for_await_binding.clone()
    }

    pub(crate) fn loop_label(&self, id: NodeId) -> Option<String> {
        self.node(id).loop_label.clone()
    }

    pub(crate) fn children(&self, id: NodeId) -> Children {
        Children {
            ids: self.node(id).children.clone(),
            pos: 0,
        }
    }

    pub(crate) fn int(&self, id: NodeId) -> Option<i64> {
        if self.kind(id) != NodeKind::IntegerLiteral {
            return None;
        }
        parse_int_literal(self.node(id).text.as_deref()?)
    }

    pub(crate) fn bool(&self, id: NodeId) -> Option<bool> {
        if self.kind(id) != NodeKind::BoolLiteral {
            return None;
        }
        match self.node(id).text.as_deref()? {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }

    pub(crate) fn float(&self, id: NodeId) -> Option<f64> {
        if self.kind(id) != NodeKind::FloatLiteral {
            return None;
        }
        parse_float_literal(self.node(id).text.as_deref()?)
    }

    pub(crate) fn param_info(&self, id: NodeId) -> ParamInfo {
        const MOD_VARIADIC: u32 = 1 << 28;
        const MOD_INOUT: u32 = 1 << 30;
        let bits = self.node(id).modifier_bits;
        ParamInfo {
            label: None,
            name: self.text(id).unwrap_or_default(),
            variadic: bits & MOD_VARIADIC != 0,
            is_inout: bits & MOD_INOUT != 0
                || self
                    .node(id)
                    .children
                    .iter()
                    .any(|&child| self.kind(child) == NodeKind::TypeInout),
        }
    }

    fn node(&self, id: NodeId) -> &RuntimeNode {
        &self.nodes[id.0]
    }
}

impl Children {
    pub(crate) fn next(&mut self) -> Option<NodeId> {
        let id = self.ids.get(self.pos).copied()?;
        self.pos += 1;
        Some(id)
    }
}

fn map_kind(kind: qswift_ast::NodeKind) -> NodeKind {
    use qswift_ast::NodeKind as K;
    match kind {
        K::SourceFile => NodeKind::SourceFile,
        K::ExprStmt => NodeKind::ExprStmt,
        K::CallExpr => NodeKind::CallExpr,
        K::IdentExpr => NodeKind::IdentExpr,
        K::BinaryExpr => NodeKind::BinaryExpr,
        K::PrefixExpr => NodeKind::UnaryExpr,
        K::AssignExpr => NodeKind::AssignExpr,
        K::TernaryExpr => NodeKind::TernaryExpr,
        K::TupleExpr => NodeKind::TupleExpr,
        K::ArrayLiteral => NodeKind::ArrayLiteral,
        K::DictLiteral => NodeKind::DictLiteral,
        K::SubscriptExpr => NodeKind::SubscriptExpr,
        K::MemberExpr => NodeKind::MemberExpr,
        K::LetDecl => NodeKind::LetDecl,
        K::VarDecl => NodeKind::VarDecl,
        K::FuncDecl => NodeKind::FuncDecl,
        K::StructDecl => NodeKind::StructDecl,
        K::EnumDecl => NodeKind::EnumDecl,
        K::ClassDecl => NodeKind::ClassDecl,
        K::ActorDecl => NodeKind::ActorDecl,
        K::ProtocolDecl => NodeKind::ProtocolDecl,
        K::ExtensionDecl => NodeKind::ExtensionDecl,
        K::AssociatedTypeDecl => NodeKind::ProtocolReq,
        K::TypeAliasDecl => NodeKind::TypealiasDecl,
        K::ImportDecl => NodeKind::ImportDecl,
        K::GenericParam => NodeKind::GenericParam,
        K::DeinitDecl => NodeKind::DeinitDecl,
        K::DoStmt => NodeKind::DoStmt,
        K::CatchClause => NodeKind::CatchClause,
        K::ThrowStmt => NodeKind::ThrowStmt,
        K::DeferStmt => NodeKind::DeferStmt,
        K::TryExpr => NodeKind::TryExpr,
        K::AwaitExpr => NodeKind::AwaitExpr,
        K::InoutExpr => NodeKind::InoutExpr,
        K::OperatorDecl => NodeKind::OperatorDecl,
        K::PrecedenceGroupDecl => NodeKind::PrecedenceGroupDecl,
        K::CompilerDirective => NodeKind::MacroExpansion,
        K::Attribute => NodeKind::Attribute,
        K::ClosureExpr => NodeKind::ClosureExpr,
        K::ClosureCapture => NodeKind::ClosureCapture,
        K::CastExpr => NodeKind::CastExpr,
        K::EnumCaseDecl => NodeKind::EnumCaseDecl,
        K::InitDecl => NodeKind::InitDecl,
        K::SubscriptDecl => NodeKind::SubscriptDecl,
        K::Accessor => NodeKind::AccessorDecl,
        K::PostfixExpr => NodeKind::ForceUnwrap,
        K::Param => NodeKind::Param,
        K::Block => NodeKind::Block,
        K::ReturnStmt => NodeKind::ReturnStmt,
        K::IfStmt => NodeKind::IfStmt,
        K::GuardStmt => NodeKind::GuardStmt,
        K::WhileStmt => NodeKind::WhileStmt,
        K::RepeatStmt => NodeKind::RepeatStmt,
        K::ForStmt => NodeKind::ForStmt,
        K::SwitchStmt => NodeKind::SwitchStmt,
        K::CaseClause => NodeKind::CaseClause,
        K::BreakStmt => NodeKind::BreakStmt,
        K::ContinueStmt => NodeKind::ContinueStmt,
        K::FallthroughStmt => NodeKind::FallthroughStmt,
        K::TypeRef => NodeKind::TypeIdent,
        K::TypeArray => NodeKind::TypeArray,
        K::TypeDict => NodeKind::TypeDict,
        K::NamePattern => NodeKind::PatternValueBinding,
        K::WildcardPattern => NodeKind::PatternWildcard,
        K::TuplePattern => NodeKind::PatternTuple,
        K::EnumCasePattern => NodeKind::PatternEnum,
        K::RangePattern => NodeKind::PatternRange,
        K::WhereClause => NodeKind::WhereClause,
        K::IntegerLiteral => NodeKind::IntegerLiteral,
        K::FloatLiteral => NodeKind::FloatLiteral,
        K::BoolLiteral => NodeKind::BoolLiteral,
        K::NilLiteral => NodeKind::NilLiteral,
        K::StringLiteral => NodeKind::StringLiteral,
    }
}

/// Translate parser modifier keywords into the runtime-facing modifier bitmask
/// (the same bit layout `quick-swift-core` reads and `modifier_names` decodes).
/// This is the frontend's stable modifier contract, independent of any C enum.
fn modifier_bits(modifiers: &[String]) -> u32 {
    let mut bits = 0u32;
    for m in modifiers {
        bits |= match m.as_str() {
            "public" => 1 << 0,
            "private" => 1 << 1,
            "internal" => 1 << 2,
            "fileprivate" => 1 << 3,
            "open" => 1 << 4,
            // `static` and a type-level `class` member both mean "static".
            "static" | "class" => 1 << 5,
            "final" => 1 << 6,
            "override" => 1 << 7,
            "mutating" => 1 << 8,
            "nonmutating" => 1 << 9,
            "lazy" => 1 << 10,
            "weak" => 1 << 11,
            "unowned" => 1 << 12,
            "async" => 1 << 13,
            "throws" => 1 << 14,
            "rethrows" => 1 << 15,
            "indirect" => 1 << 16,
            "required" => 1 << 17,
            "convenience" => 1 << 18,
            "dynamic" => 1 << 19,
            // Parameter flags: `T...` variadic and `inout`. The runtime reads
            // variadic via the same 1<<28 bit; inout uses a frontend-internal
            // bit surfaced through `param_info`.
            "variadic" => 1 << 28,
            "inout" => 1 << 30,
            _ => 0,
        };
    }
    bits
}

fn parse_int_literal(text: &str) -> Option<i64> {
    let mut s = text.replace('_', "");
    let negative = s.starts_with('-');
    if negative {
        s.remove(0);
    }
    let (digits, radix) = if let Some(rest) = s.strip_prefix("0x") {
        (rest, 16)
    } else if let Some(rest) = s.strip_prefix("0b") {
        (rest, 2)
    } else if let Some(rest) = s.strip_prefix("0o") {
        (rest, 8)
    } else {
        (s.as_str(), 10)
    };
    let value = i64::from_str_radix(digits, radix).ok()?;
    Some(if negative { -value } else { value })
}

/// Parse a Swift floating-point literal, including the hexadecimal form
/// (`0x1.8p1`) that Rust's `str::parse::<f64>` rejects.
fn parse_float_literal(text: &str) -> Option<f64> {
    let s = text.replace('_', "");
    let body = s.strip_prefix('-').unwrap_or(&s);
    let value = if let Some(rest) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        parse_hex_float(rest)?
    } else {
        body.parse().ok()?
    };
    Some(if s.starts_with('-') { -value } else { value })
}

/// Parse the mantissa/exponent of a hex float (the part after `0x`): hexadecimal
/// `int[.frac]` scaled by a binary exponent `p[±]dec`, e.g. `1.8p1` → 3.0.
fn parse_hex_float(rest: &str) -> Option<f64> {
    let p = rest.find(['p', 'P'])?;
    let mantissa = &rest[..p];
    let exponent: i32 = rest[p + 1..].parse().ok()?;
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));

    let mut value = 0.0f64;
    for c in int_part.chars() {
        value = value * 16.0 + f64::from(c.to_digit(16)?);
    }
    let mut scale = 1.0 / 16.0;
    for c in frac_part.chars() {
        value += f64::from(c.to_digit(16)?) * scale;
        scale /= 16.0;
    }
    Some(value * 2f64.powi(exponent))
}

#[cfg(test)]
mod tests {
    use super::{parse_float_literal, parse_int_literal};

    #[test]
    fn integer_literals_in_every_radix() {
        assert_eq!(parse_int_literal("1_000"), Some(1000));
        assert_eq!(parse_int_literal("0xFF"), Some(255));
        assert_eq!(parse_int_literal("0o755"), Some(493));
        assert_eq!(parse_int_literal("0b1010"), Some(10));
        assert_eq!(parse_int_literal("-42"), Some(-42));
    }

    #[test]
    fn decimal_floats_parse() {
        assert_eq!(parse_float_literal("3.5"), Some(3.5));
        assert_eq!(parse_float_literal("1.5e3"), Some(1500.0));
        assert_eq!(parse_float_literal("1_000.5"), Some(1000.5));
    }

    #[test]
    fn hex_floats_parse() {
        assert_eq!(parse_float_literal("0x1.8p1"), Some(3.0));
        assert_eq!(parse_float_literal("0x1p4"), Some(16.0));
        assert_eq!(parse_float_literal("0xA.8p0"), Some(10.5));
        assert_eq!(parse_float_literal("-0x1.0p2"), Some(-4.0));
    }
}
