//! The result-builder transform pass.
//!
//! A `@Builder`-annotated function body is a *result-builder DSL*: a sequence of
//! component expressions the builder folds into one value (SE-0289). This pass
//! performs that fold at compile time, rewriting the body into ordinary
//! `Builder.buildBlock(...)` method calls and **erasing** the builder attribute,
//! so the interpreter sees plain static calls and needs no builder-specific
//! evaluation.
//!
//! This is the walking skeleton (issue #119): it handles a body of declarations
//! and expression statements with `buildExpression` + `buildBlock`. A body
//! containing control flow it does not yet lower is **left untouched** (attribute
//! intact) so the legacy runtime transform still handles it — keeping every
//! fixture green while later slices grow the handled set.

use tswift_ast::{Ast, NodeId, NodeKind};

use crate::astbuild;
use crate::passes::Pass;
use crate::symbols::{BuilderMethods, Symbols};
use crate::{child_ids, Diagnostic};

/// Rewrite builder-bodied functions into ordinary build-method calls.
pub(crate) struct BuilderTransform;

impl Pass for BuilderTransform {
    fn run(&self, ast: &mut Ast, symbols: &Symbols) -> Vec<Diagnostic> {
        let mut targets = Vec::new();
        collect_targets(ast, ast.root(), symbols, &mut targets);
        for (func, builder) in targets {
            transform_func(ast, func, &builder, symbols);
        }
        Vec::new()
    }
}

/// Find every `FuncDecl` carrying a result-builder attribute, paired with the
/// builder type's name. Recurses into nested scopes so builder *methods* (funcs
/// declared inside a type) are found too.
fn collect_targets(ast: &Ast, parent: NodeId, symbols: &Symbols, out: &mut Vec<(NodeId, String)>) {
    for child in child_ids(ast, parent) {
        if ast.node(child).kind() == NodeKind::FuncDecl {
            if let Some(builder) = builder_attr(ast, child, symbols) {
                out.push((child, builder));
            }
        }
        collect_targets(ast, child, symbols, out);
    }
}

/// The name of the result builder named by one of `func`'s attributes, if any.
fn builder_attr(ast: &Ast, func: NodeId, symbols: &Symbols) -> Option<String> {
    ast.node(func)
        .children()
        .filter(|c| c.kind() == NodeKind::Attribute)
        .filter_map(|c| c.text())
        .find(|name| symbols.result_builder(name).is_some())
        .map(str::to_string)
}

/// Transform `func`'s body if it is fully handleable; otherwise leave it (and
/// its attribute) untouched for the legacy runtime transform.
fn transform_func(ast: &mut Ast, func: NodeId, builder: &str, symbols: &Symbols) {
    let Some(methods) = symbols.result_builder(builder) else {
        return;
    };
    // The builder must declare `buildBlock` (the required fold method).
    if !methods.has("buildBlock") {
        return;
    }
    let Some(body) = child_ids(ast, func)
        .into_iter()
        .find(|c| ast.node(*c).kind() == NodeKind::Block)
    else {
        return;
    };
    let stmts = child_ids(ast, body);
    if !stmts.iter().all(|&s| is_handleable(ast, s)) {
        return; // contains a construct a later slice will lower; defer.
    }

    let new_body = rewrite_body(ast, body, &stmts, builder, methods);
    ast.set_children(body, new_body);
    erase_attribute(ast, func, builder);
}

/// Whether the skeleton can lower statement `stmt`: a declaration (kept in
/// place) or a bare expression statement (a component). Control flow is deferred
/// to later slices.
fn is_handleable(ast: &Ast, stmt: NodeId) -> bool {
    matches!(
        ast.node(stmt).kind(),
        NodeKind::ExprStmt | NodeKind::LetDecl | NodeKind::VarDecl | NodeKind::FuncDecl
    )
}

/// Build the rewritten statement list: declarations passed through, each
/// expression statement bound to a fresh `$buildN`, and a trailing
/// `return Builder.buildBlock($build0, …)`.
fn rewrite_body(
    ast: &mut Ast,
    body: NodeId,
    stmts: &[NodeId],
    builder: &str,
    methods: &BuilderMethods,
) -> Vec<NodeId> {
    let has_expr_hook = methods.has("buildExpression");
    let mut new_stmts = Vec::new();
    let mut components = Vec::new();
    for &stmt in stmts {
        if ast.node(stmt).kind() != NodeKind::ExprStmt {
            new_stmts.push(stmt); // declaration: leave in place.
            continue;
        }
        let (line, col) = (ast.node(stmt).line(), ast.node(stmt).col());
        // The expression statement's sole child is the component expression.
        let expr = child_ids(ast, stmt)[0];
        let arg = if has_expr_hook {
            astbuild::static_call(
                ast,
                builder,
                "buildExpression",
                vec![(None, expr)],
                line,
                col,
            )
        } else {
            expr
        };
        let name = astbuild::fresh_name(components.len());
        let decl = astbuild::fresh_let(ast, &name, arg, line, col);
        new_stmts.push(decl);
        components.push(name);
    }

    let (line, col) = (ast.node(body).line(), ast.node(body).col());
    let args: Vec<(Option<&str>, NodeId)> = components
        .iter()
        .map(|name| (None, astbuild::ident(ast, name, line, col)))
        .collect();
    let block_call = astbuild::static_call(ast, builder, "buildBlock", args, line, col);
    new_stmts.push(astbuild::return_stmt(ast, block_call, line, col));
    new_stmts
}

/// Drop the `@builder` attribute child from `func`, so the interpreter no longer
/// treats it as a builder body (it is now plain method calls).
fn erase_attribute(ast: &mut Ast, func: NodeId, builder: &str) {
    let kept: Vec<NodeId> = child_ids(ast, func)
        .into_iter()
        .filter(|&c| {
            !(ast.node(c).kind() == NodeKind::Attribute && ast.node(c).text() == Some(builder))
        })
        .collect();
    ast.set_children(func, kept);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze;
    use tswift_parser::parse;

    /// Find the rewritten body block of the function named `name`.
    fn func_body<'a>(ast: &'a Ast, name: &str) -> tswift_ast::Node<'a> {
        fn find(ast: &Ast, parent: NodeId, name: &str) -> Option<NodeId> {
            for c in child_ids(ast, parent) {
                if ast.node(c).kind() == NodeKind::FuncDecl && ast.node(c).text() == Some(name) {
                    return Some(c);
                }
                if let Some(f) = find(ast, c, name) {
                    return Some(f);
                }
            }
            None
        }
        let func = find(ast, ast.root(), name).expect("function present");
        ast.node(func)
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .expect("body block")
    }

    const STRING_BUILDER: &str = "@resultBuilder\nstruct B {\n\
        static func buildExpression(_ v: String) -> String { v }\n\
        static func buildBlock(_ parts: String...) -> String { \"\" }\n}\n";

    fn analyzed(extra: &str) -> Ast {
        let mut ast = parse(&format!("{STRING_BUILDER}{extra}")).expect("parse ok");
        let diags = analyze(&mut ast);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        ast
    }

    #[test]
    fn rewrites_expression_body_to_build_block_calls() {
        let ast = analyzed("@B\nfunc g() -> String {\n \"a\"\n \"b\"\n}");
        let body = func_body(&ast, "g");
        let kids: Vec<_> = body.children().map(|c| c.kind()).collect();
        // two synthesized lets + a return
        assert_eq!(
            kids,
            vec![NodeKind::LetDecl, NodeKind::LetDecl, NodeKind::ReturnStmt]
        );

        // Each let binds a `$buildN` to `B.buildExpression(<literal>)`.
        let first = body.children().next().unwrap();
        assert_eq!(first.children().next().unwrap().text(), Some("$build0"));
        let call = first.children().nth(1).unwrap();
        assert_eq!(call.kind(), NodeKind::CallExpr);
        let callee = call.children().next().unwrap();
        assert_eq!(callee.kind(), NodeKind::MemberExpr);
        assert_eq!(callee.text(), Some("buildExpression"));

        // The return is `B.buildBlock($build0, $build1)`.
        let ret = body.children().last().unwrap();
        let ret_call = ret.children().next().unwrap();
        assert_eq!(
            ret_call.children().next().unwrap().text(),
            Some("buildBlock")
        );
        let arg_names: Vec<_> = ret_call.children().skip(1).map(|c| c.text()).collect();
        assert_eq!(arg_names, vec![Some("$build0"), Some("$build1")]);
    }

    #[test]
    fn erases_the_builder_attribute() {
        let ast = analyzed("@B\nfunc g() -> String {\n \"a\"\n}");
        fn find_func(ast: &Ast, parent: NodeId) -> Option<NodeId> {
            for c in child_ids(ast, parent) {
                if ast.node(c).kind() == NodeKind::FuncDecl && ast.node(c).text() == Some("g") {
                    return Some(c);
                }
                if let Some(f) = find_func(ast, c) {
                    return Some(f);
                }
            }
            None
        }
        let func = find_func(&ast, ast.root()).unwrap();
        assert!(
            !ast.node(func)
                .children()
                .any(|c| c.kind() == NodeKind::Attribute),
            "builder attribute should be erased after transform"
        );
    }

    #[test]
    fn declarations_stay_in_place_and_remain_components_sources() {
        let ast = analyzed("@B\nfunc g() -> String {\n let p = \"Hi\"\n p\n}");
        let body = func_body(&ast, "g");
        let kids: Vec<_> = body.children().map(|c| c.kind()).collect();
        // user let kept, one synthesized let for `p`, the return
        assert_eq!(
            kids,
            vec![NodeKind::LetDecl, NodeKind::LetDecl, NodeKind::ReturnStmt]
        );
        let user_let = body.children().next().unwrap();
        assert_eq!(user_let.children().next().unwrap().text(), Some("p"));
    }

    #[test]
    fn empty_body_folds_to_an_empty_build_block() {
        let ast = analyzed("@B\nfunc g() -> String {\n}");
        let body = func_body(&ast, "g");
        let kids: Vec<_> = body.children().map(|c| c.kind()).collect();
        assert_eq!(kids, vec![NodeKind::ReturnStmt]);
        let ret_call = body.children().next().unwrap().children().next().unwrap();
        assert_eq!(
            ret_call.children().next().unwrap().text(),
            Some("buildBlock")
        );
        assert_eq!(ret_call.children().count(), 1, "buildBlock() takes no args");
    }

    #[test]
    fn body_without_expression_hook_passes_components_through() {
        let src = "@resultBuilder\nstruct C {\n\
            static func buildBlock(_ parts: String...) -> String { \"\" }\n}\n\
            @C\nfunc g() -> String {\n \"a\"\n}";
        let mut ast = parse(src).expect("parse ok");
        analyze(&mut ast);
        let body = func_body(&ast, "g");
        // No buildExpression: the let binds the literal directly.
        let first = body.children().next().unwrap();
        assert_eq!(
            first.children().nth(1).unwrap().kind(),
            NodeKind::StringLiteral
        );
    }

    #[test]
    fn control_flow_body_is_left_for_the_runtime_path() {
        // An `if` is not yet handled, so the body and its attribute are intact.
        let ast = analyzed("@B\nfunc g(_ f: Bool) -> String {\n \"a\"\n if f { \"b\" }\n}");
        fn find_func(ast: &Ast, parent: NodeId) -> Option<NodeId> {
            for c in child_ids(ast, parent) {
                if ast.node(c).kind() == NodeKind::FuncDecl && ast.node(c).text() == Some("g") {
                    return Some(c);
                }
                if let Some(f) = find_func(ast, c) {
                    return Some(f);
                }
            }
            None
        }
        let func = find_func(&ast, ast.root()).unwrap();
        assert!(
            ast.node(func)
                .children()
                .any(|c| c.kind() == NodeKind::Attribute),
            "unhandled body keeps its builder attribute"
        );
        // Body still has the original `if` statement.
        let body = func_body(&ast, "g");
        assert!(body.children().any(|c| c.kind() == NodeKind::IfStmt));
    }
}
