//! Rust-backend compatibility tree for the runtime-facing frontend facade.
//!
//! This module is intentionally internal: `quick-swift-core` keeps consuming
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
    children: Vec<NodeId>,
}

pub(crate) struct Children {
    ids: Vec<NodeId>,
    pos: usize,
}

impl RuntimeAst {
    pub(crate) fn analyze(source: &str) -> RuntimeAst {
        let mut ast = match swift_parser::parse(source) {
            Ok(ast) => ast,
            Err(e) => return RuntimeAst::diagnostic(e.message, e.line, e.col),
        };
        let diagnostics = swift_sema::resolve(&mut ast)
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
                children: Vec::new(),
            }],
            diagnostics: vec![Diagnostic { message, line, col }],
        }
    }

    fn lower_node(&mut self, node: swift_ast::Node<'_>) -> NodeId {
        let id = NodeId(self.nodes.len());
        let kind = map_kind(node.kind());
        self.nodes.push(RuntimeNode {
            kind,
            text: node.text().map(ToOwned::to_owned),
            line: node.line(),
            ty: node.type_name().map(ToOwned::to_owned),
            children: Vec::new(),
        });

        let children: Vec<NodeId> = node
            .children()
            .map(|child| self.lower_node(child))
            .collect();
        let inferred_decl_name = if matches!(kind, NodeKind::LetDecl | NodeKind::VarDecl) {
            children
                .iter()
                .find_map(|&child| self.node(child).text.clone())
        } else {
            None
        };
        let out = &mut self.nodes[id.0];
        out.children = children;
        if out.text.is_none() {
            out.text = inferred_decl_name;
        }
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
        self.node(id).text.as_deref()?.replace('_', "").parse().ok()
    }

    pub(crate) fn param_info(&self, id: NodeId) -> ParamInfo {
        ParamInfo {
            label: None,
            name: self.text(id).unwrap_or_default(),
            variadic: false,
            is_inout: self
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

fn map_kind(kind: swift_ast::NodeKind) -> NodeKind {
    use swift_ast::NodeKind as K;
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
        K::MemberExpr => NodeKind::MemberExpr,
        K::LetDecl => NodeKind::LetDecl,
        K::VarDecl => NodeKind::VarDecl,
        K::FuncDecl => NodeKind::FuncDecl,
        K::StructDecl => NodeKind::StructDecl,
        K::EnumDecl => NodeKind::EnumDecl,
        K::ClassDecl => NodeKind::ClassDecl,
        K::ProtocolDecl => NodeKind::ProtocolDecl,
        K::ExtensionDecl => NodeKind::ExtensionDecl,
        K::AssociatedTypeDecl => NodeKind::ProtocolReq,
        K::TypeAliasDecl => NodeKind::TypealiasDecl,
        K::GenericParam => NodeKind::GenericParam,
        K::DeinitDecl => NodeKind::DeinitDecl,
        K::DoStmt => NodeKind::DoStmt,
        K::CatchClause => NodeKind::CatchClause,
        K::ThrowStmt => NodeKind::ThrowStmt,
        K::DeferStmt => NodeKind::DeferStmt,
        K::TryExpr => NodeKind::TryExpr,
        K::OperatorDecl => NodeKind::OperatorDecl,
        K::PrecedenceGroupDecl => NodeKind::PrecedenceGroupDecl,
        K::CompilerDirective => NodeKind::MacroExpansion,
        K::Attribute => NodeKind::Attribute,
        K::ClosureExpr => NodeKind::ClosureExpr,
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
        K::NamePattern => NodeKind::PatternValueBinding,
        K::WildcardPattern => NodeKind::PatternWildcard,
        K::TuplePattern => NodeKind::PatternTuple,
        K::IntegerLiteral => NodeKind::IntegerLiteral,
        K::FloatLiteral => NodeKind::FloatLiteral,
        K::BoolLiteral => NodeKind::BoolLiteral,
        K::NilLiteral => NodeKind::NilLiteral,
        K::StringLiteral => NodeKind::StringLiteral,
    }
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
