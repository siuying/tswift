//! Semantic analysis for the quick-swift frontend.
//!
//! [`resolve`] walks a parsed [`swift_ast::Ast`], infers and records a [`Type`]
//! on each expression node, and returns any [`Diagnostic`]s. Scope today is the
//! walking-skeleton subset: literal types, arithmetic result types, and the
//! `Void` result of a `print(...)` call — enough to type `print("hi")` and
//! integer arithmetic. Name resolution and conformance arrive in later tiers.

#![forbid(unsafe_code)]

use swift_ast::{Ast, NodeId, NodeKind, Type};

/// One semantic diagnostic with its 1-based source location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub line: u32,
    pub col: u32,
}

/// Resolve types over `ast` in place, returning diagnostics in source order.
pub fn resolve(ast: &mut Ast) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let mut types: Vec<(NodeId, Type)> = Vec::new();
    let root = ast.root();
    infer(ast, root, &mut types, &mut diags);
    for (id, ty) in types {
        ast.set_type(id, ty);
    }
    diags
}

/// Infer the type of `id` (post-order), pushing `(node, type)` assignments to
/// apply afterwards so we never hold a mutable borrow during traversal.
fn infer(
    ast: &Ast,
    id: NodeId,
    types: &mut Vec<(NodeId, Type)>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Type> {
    let node = ast.node(id);
    let children: Vec<NodeId> = node.children().map(|c| c.id()).collect();
    // Resolve children first.
    let child_types: Vec<Option<Type>> = children
        .iter()
        .map(|&c| infer(ast, c, types, diags))
        .collect();

    let ty = match node.kind() {
        NodeKind::IntegerLiteral => Some(Type::Int),
        NodeKind::StringLiteral => Some(Type::String),
        NodeKind::BinaryExpr => infer_binary(&node, &child_types, diags),
        // `print(...)` returns Void; other callees are unresolved for now.
        NodeKind::CallExpr => Some(Type::Void),
        // Identifiers, statements, and the file node carry no expression type yet.
        NodeKind::IdentExpr | NodeKind::ExprStmt | NodeKind::SourceFile => None,
    };
    if let Some(t) = ty {
        types.push((id, t));
    }
    ty
}

fn infer_binary(
    node: &swift_ast::Node<'_>,
    child_types: &[Option<Type>],
    diags: &mut Vec<Diagnostic>,
) -> Option<Type> {
    let lhs = child_types.first().copied().flatten();
    let rhs = child_types.get(1).copied().flatten();
    match (lhs, rhs) {
        (Some(a), Some(b)) if a == b => Some(a),
        (Some(a), Some(b)) => {
            diags.push(Diagnostic {
                message: format!(
                    "binary operator '{}' cannot combine {} and {}",
                    node.text().unwrap_or("?"),
                    a.name(),
                    b.name()
                ),
                line: node.line(),
                col: node.col(),
            });
            Some(a) // recover with the left type
        }
        // Unknown operand types: leave unresolved without erroring.
        _ => lhs.or(rhs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swift_parser::parse;

    fn resolved(src: &str) -> (Ast, Vec<Diagnostic>) {
        let mut ast = parse(src).expect("parse ok");
        let diags = resolve(&mut ast);
        (ast, diags)
    }

    #[test]
    fn types_integer_arithmetic_as_int() {
        let (ast, diags) = resolved("1 + 2");
        assert!(diags.is_empty(), "{diags:?}");
        // source_file > expr_stmt > binary_expr
        let bin = ast
            .node(ast.root())
            .children()
            .next()
            .unwrap()
            .children()
            .next()
            .unwrap();
        assert_eq!(bin.type_name(), Some("Int"));
    }

    #[test]
    fn print_call_is_void_and_clean() {
        let (ast, diags) = resolved(r#"print("hi")"#);
        assert!(diags.is_empty(), "{diags:?}");
        let call = ast
            .node(ast.root())
            .children()
            .next()
            .unwrap()
            .children()
            .next()
            .unwrap();
        assert_eq!(call.kind(), NodeKind::CallExpr);
        assert_eq!(call.type_name(), Some("Void"));
        // The string argument is typed String.
        let arg = call.children().nth(1).unwrap();
        assert_eq!(arg.type_name(), Some("String"));
    }

    #[test]
    fn mixed_operand_types_diagnose() {
        let (_ast, diags) = resolved(r#"1 + "x""#);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("cannot combine"), "{diags:?}");
        assert_eq!(diags[0].line, 1);
    }
}
