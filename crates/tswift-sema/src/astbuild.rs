//! Arena node synthesis for AST→AST rewrites.
//!
//! The result-builder transform ([`crate::builder_transform`]) rebuilds a
//! builder body into ordinary `Builder.buildBlock(...)` method calls. These
//! helpers construct the handful of node shapes that rewrite needs directly in
//! the [`Ast`] arena, via the public mutation API (`add` / `append_child` /
//! `set_arg_label`), so the transform reads as "what tree am I building" rather
//! than arena bookkeeping.
//!
//! Synthesized bindings are named with [`fresh_name`], which prefixes a reserved
//! `$build` the lexer cannot produce — so a synthesized name can never collide
//! with a user identifier.

use tswift_ast::{Ast, NodeId, NodeKind};

/// The reserved prefix for synthesized result-builder bindings. The lexer never
/// produces a `$`-leading identifier, so these names cannot shadow user names.
pub(crate) const SYNTH_PREFIX: &str = "$build";

/// Mint the `n`-th synthesized binding name (`$build0`, `$build1`, …).
pub(crate) fn fresh_name(n: usize) -> String {
    format!("{SYNTH_PREFIX}{n}")
}

/// An identifier-reference expression, `name`.
pub(crate) fn ident(ast: &mut Ast, name: &str, line: u32, col: u32) -> NodeId {
    ast.add(NodeKind::IdentExpr, Some(name), line, col)
}

/// A member access `base.member`. The `MemberExpr`'s text is the member name and
/// its sole child is the base expression — the shape the parser produces.
pub(crate) fn member(ast: &mut Ast, base: NodeId, name: &str, line: u32, col: u32) -> NodeId {
    let m = ast.add(NodeKind::MemberExpr, Some(name), line, col);
    ast.append_child(m, base);
    m
}

/// A static builder-method call `Builder.method(args...)`.
///
/// Each argument is `(optional_label, expr)`; a label is recorded with
/// [`Ast::set_arg_label`] so `(first: x)` round-trips to `buildEither(first:)`.
pub(crate) fn static_call(
    ast: &mut Ast,
    builder: &str,
    method: &str,
    args: Vec<(Option<&str>, NodeId)>,
    line: u32,
    col: u32,
) -> NodeId {
    let base = ident(ast, builder, line, col);
    let callee = member(ast, base, method, line, col);
    let call = ast.add(NodeKind::CallExpr, None, line, col);
    ast.append_child(call, callee);
    for (label, arg) in args {
        if let Some(label) = label {
            ast.set_arg_label(arg, label);
        }
        ast.append_child(call, arg);
    }
    call
}

/// A `let name = expr` binding declaration.
pub(crate) fn fresh_let(ast: &mut Ast, name: &str, expr: NodeId, line: u32, col: u32) -> NodeId {
    let decl = ast.add(NodeKind::LetDecl, None, line, col);
    let pattern = ast.add(NodeKind::NamePattern, Some(name), line, col);
    ast.append_child(decl, pattern);
    ast.append_child(decl, expr);
    decl
}

/// A `return expr` statement.
pub(crate) fn return_stmt(ast: &mut Ast, expr: NodeId, line: u32, col: u32) -> NodeId {
    let ret = ast.add(NodeKind::ReturnStmt, None, line, col);
    ast.append_child(ret, expr);
    ret
}

/// An uninitialized `var name` declaration. The result-builder transform uses
/// it to hold a conditional component, assigned in each branch of a synthesized
/// `if` (definite-initialization holds because every branch assigns it).
pub(crate) fn var_decl(ast: &mut Ast, name: &str, line: u32, col: u32) -> NodeId {
    let decl = ast.add(NodeKind::VarDecl, None, line, col);
    let pattern = ast.add(NodeKind::NamePattern, Some(name), line, col);
    ast.append_child(decl, pattern);
    decl
}

/// An assignment statement `target = value` (used as a block statement, which is
/// how the parser represents a bare assignment — not wrapped in an `ExprStmt`).
pub(crate) fn assign(ast: &mut Ast, target: &str, value: NodeId, line: u32, col: u32) -> NodeId {
    let node = ast.add(NodeKind::AssignExpr, Some("="), line, col);
    let lhs = ident(ast, target, line, col);
    ast.append_child(node, lhs);
    ast.append_child(node, value);
    node
}

/// The `nil` literal.
pub(crate) fn nil_literal(ast: &mut Ast, line: u32, col: u32) -> NodeId {
    ast.add(NodeKind::NilLiteral, None, line, col)
}

/// A `var name = init` declaration (a mutable binding with an initializer),
/// used for the `for`-loop accumulator array.
pub(crate) fn var_decl_init(
    ast: &mut Ast,
    name: &str,
    init: NodeId,
    line: u32,
    col: u32,
) -> NodeId {
    let decl = ast.add(NodeKind::VarDecl, None, line, col);
    let pattern = ast.add(NodeKind::NamePattern, Some(name), line, col);
    ast.append_child(decl, pattern);
    ast.append_child(decl, init);
    decl
}

/// An empty array literal `[]`.
pub(crate) fn empty_array(ast: &mut Ast, line: u32, col: u32) -> NodeId {
    ast.add(NodeKind::ArrayLiteral, None, line, col)
}

/// An instance-method call `receiver.method(args...)`.
pub(crate) fn method_call(
    ast: &mut Ast,
    receiver: NodeId,
    method: &str,
    args: Vec<NodeId>,
    line: u32,
    col: u32,
) -> NodeId {
    let callee = member(ast, receiver, method, line, col);
    let call = ast.add(NodeKind::CallExpr, None, line, col);
    ast.append_child(call, callee);
    for arg in args {
        ast.append_child(call, arg);
    }
    call
}

/// A braced statement block holding `stmts`.
pub(crate) fn block(ast: &mut Ast, stmts: Vec<NodeId>, line: u32, col: u32) -> NodeId {
    let block = ast.add(NodeKind::Block, None, line, col);
    for s in stmts {
        ast.append_child(block, s);
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_names_use_the_reserved_prefix() {
        assert_eq!(fresh_name(0), "$build0");
        assert_eq!(fresh_name(7), "$build7");
        assert!(fresh_name(0).starts_with(SYNTH_PREFIX));
    }

    #[test]
    fn static_call_builds_member_callee_and_labelled_args() {
        let mut ast = Ast::new();
        let arg = ast.add(NodeKind::IdentExpr, Some("x"), 1, 1);
        let call = static_call(
            &mut ast,
            "B",
            "buildEither",
            vec![(Some("first"), arg)],
            1,
            1,
        );

        let node = ast.node(call);
        assert_eq!(node.kind(), NodeKind::CallExpr);
        let mut kids = node.children();
        let callee = kids.next().unwrap();
        assert_eq!(callee.kind(), NodeKind::MemberExpr);
        assert_eq!(callee.text(), Some("buildEither"));
        assert_eq!(
            callee.children().next().unwrap().text(),
            Some("B"),
            "callee base is the builder type"
        );
        let arg = kids.next().unwrap();
        assert_eq!(arg.text(), Some("x"));
        assert_eq!(arg.arg_label(), Some("first"));
    }

    #[test]
    fn fresh_let_binds_a_name_pattern_to_an_expr() {
        let mut ast = Ast::new();
        let expr = ast.add(NodeKind::StringLiteral, Some("\"a\""), 1, 1);
        let decl = fresh_let(&mut ast, "$build0", expr, 1, 1);

        let node = ast.node(decl);
        assert_eq!(node.kind(), NodeKind::LetDecl);
        let pattern = node.children().next().unwrap();
        assert_eq!(pattern.kind(), NodeKind::NamePattern);
        assert_eq!(pattern.text(), Some("$build0"));
        assert_eq!(
            node.children().nth(1).unwrap().kind(),
            NodeKind::StringLiteral
        );
    }

    #[test]
    fn return_stmt_wraps_its_value() {
        let mut ast = Ast::new();
        let v = ast.add(NodeKind::IdentExpr, Some("v"), 1, 1);
        let ret = return_stmt(&mut ast, v, 1, 1);
        let node = ast.node(ret);
        assert_eq!(node.kind(), NodeKind::ReturnStmt);
        assert_eq!(node.children().next().unwrap().text(), Some("v"));
    }
}
