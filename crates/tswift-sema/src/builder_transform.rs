//! The result-builder transform pass.
//!
//! A `@Builder`-annotated function body is a *result-builder DSL*: a sequence of
//! component expressions the builder folds into one value (SE-0289). This pass
//! performs that fold at compile time, rewriting the body into ordinary
//! `Builder.buildBlock(...)` method calls and **erasing** the builder attribute,
//! so the interpreter sees plain static calls and needs no builder-specific
//! evaluation.
//!
//! Handled so far: declarations and expression statements (`buildExpression` +
//! `buildBlock`, #119) and `if`/`else`/`if let` conditionals (`buildEither` +
//! `buildOptional`, #120). A body containing control flow not yet lowered is
//! **left untouched** (attribute intact) so the legacy runtime transform still
//! handles it — keeping every fixture green while later slices grow the set.

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
    if !stmts.iter().all(|&s| is_handleable(ast, methods, s)) {
        return; // contains a construct a later slice will lower; defer.
    }

    let (line, col) = (ast.node(body).line(), ast.node(body).col());
    let mut lowering = Lowering {
        ast,
        builder,
        methods,
        counter: 0,
    };
    let (mut new_body, value) = lowering.lower_block(&stmts, line, col);
    let ret = astbuild::return_stmt(lowering.ast, value, line, col);
    new_body.push(ret);
    ast.set_children(body, new_body);
    erase_attribute(ast, func, builder);
}

/// Whether the transform can lower statement `stmt` (recursively): a declaration
/// (kept in place), a bare expression statement (a component), an `if` whose
/// branches are handleable (and whose builder declares the needed `buildEither`/
/// `buildOptional`), or a `for` whose builder declares `buildArray`. Other
/// control flow defers the whole function to the runtime path.
fn is_handleable(ast: &Ast, methods: &BuilderMethods, stmt: NodeId) -> bool {
    match ast.node(stmt).kind() {
        NodeKind::ExprStmt | NodeKind::LetDecl | NodeKind::VarDecl | NodeKind::FuncDecl => true,
        NodeKind::IfStmt => {
            // A conditional needs at least one of the selection methods; without
            // them the body is invalid Swift, so leave it to the runtime path.
            (methods.has("buildEither") || methods.has("buildOptional"))
                && child_ids(ast, stmt)
                    .into_iter()
                    .all(|c| match ast.node(c).kind() {
                        NodeKind::Block => child_ids(ast, c)
                            .iter()
                            .all(|&s| is_handleable(ast, methods, s)),
                        NodeKind::IfStmt => is_handleable(ast, methods, c),
                        _ => true,
                    })
        }
        NodeKind::ForStmt => {
            methods.has("buildArray")
                && child_ids(ast, stmt).into_iter().all(|c| {
                    ast.node(c).kind() != NodeKind::Block
                        || child_ids(ast, c)
                            .iter()
                            .all(|&s| is_handleable(ast, methods, s))
                })
        }
        _ => false,
    }
}

/// The recursive lowering state: the arena, the builder being lowered, its
/// method set, and a monotonic counter for fresh `$buildN` names.
struct Lowering<'a> {
    ast: &'a mut Ast,
    builder: &'a str,
    methods: &'a BuilderMethods,
    counter: usize,
}

impl Lowering<'_> {
    /// Mint the next `$buildN` name.
    fn fresh(&mut self) -> String {
        let name = astbuild::fresh_name(self.counter);
        self.counter += 1;
        name
    }

    /// Lower a builder statement sequence. Returns the statements to emit and the
    /// expression for the block's folded value (`Builder.buildBlock(c0, …)`).
    /// Each component value is bound to a fresh variable so control-flow bodies
    /// run as real statements.
    fn lower_block(&mut self, stmts: &[NodeId], line: u32, col: u32) -> (Vec<NodeId>, NodeId) {
        let mut out = Vec::new();
        let mut components = Vec::new();
        for &stmt in stmts {
            match self.ast.node(stmt).kind() {
                NodeKind::ExprStmt => {
                    let (l, c) = (self.ast.node(stmt).line(), self.ast.node(stmt).col());
                    let expr = child_ids(self.ast, stmt)[0];
                    let arg = if self.methods.has("buildExpression") {
                        astbuild::static_call(
                            self.ast,
                            self.builder,
                            "buildExpression",
                            vec![(None, expr)],
                            l,
                            c,
                        )
                    } else {
                        expr
                    };
                    let name = self.fresh();
                    out.push(astbuild::fresh_let(self.ast, &name, arg, l, c));
                    components.push(name);
                }
                NodeKind::IfStmt => {
                    let (stmts, name) = self.lower_if(stmt);
                    out.extend(stmts);
                    components.push(name);
                }
                NodeKind::ForStmt => {
                    let (stmts, name) = self.lower_for(stmt);
                    out.extend(stmts);
                    components.push(name);
                }
                // A declaration is not a component: leave it in place.
                _ => out.push(stmt),
            }
        }
        let args: Vec<(Option<&str>, NodeId)> = components
            .iter()
            .map(|name| (None, astbuild::ident(self.ast, name, line, col)))
            .collect();
        let value = astbuild::static_call(self.ast, self.builder, "buildBlock", args, line, col);
        (out, value)
    }

    /// Lower an `if` to a fresh component var assigned in every branch. Returns
    /// `(var-decl + rebuilt-if, component-name)`.
    ///
    /// `if c { A } else { B }` → `buildEither(first:)` / `buildEither(second:)`;
    /// a bare `if c { A }` → `buildOptional` (value when taken, `nil` otherwise);
    /// an `else if` nests as a further `buildEither(second:)` over the chain.
    fn lower_if(&mut self, if_node: NodeId) -> (Vec<NodeId>, String) {
        let comp = self.fresh();
        let (line, col) = (self.ast.node(if_node).line(), self.ast.node(if_node).col());
        let kids = child_ids(self.ast, if_node);
        let then_idx = kids
            .iter()
            .position(|&c| self.ast.node(c).kind() == NodeKind::Block)
            .expect("if has a then-block");
        let conds = kids[..then_idx].to_vec();
        let then_block = kids[then_idx];
        let els = kids.get(then_idx + 1).copied();
        let has_else = els.is_some();

        // Then branch: lower its block, then assign the chosen component value.
        let then_stmts = child_ids(self.ast, then_block);
        let (mut tstmts, tvalue) = self.lower_block(&then_stmts, line, col);
        let then_value = if has_else {
            astbuild::static_call(
                self.ast,
                self.builder,
                "buildEither",
                vec![(Some("first"), tvalue)],
                line,
                col,
            )
        } else {
            self.build_optional(tvalue, line, col)
        };
        tstmts.push(astbuild::assign(self.ast, &comp, then_value, line, col));
        let new_then = astbuild::block(self.ast, tstmts, line, col);

        // Else branch: a block, an `else if` chain, or a synthesized `nil` arm.
        let new_else = match els.map(|e| (e, self.ast.node(e).kind())) {
            Some((e, NodeKind::Block)) => {
                let estmts = child_ids(self.ast, e);
                let (mut estmts, evalue) = self.lower_block(&estmts, line, col);
                let value = astbuild::static_call(
                    self.ast,
                    self.builder,
                    "buildEither",
                    vec![(Some("second"), evalue)],
                    line,
                    col,
                );
                estmts.push(astbuild::assign(self.ast, &comp, value, line, col));
                astbuild::block(self.ast, estmts, line, col)
            }
            Some((e, NodeKind::IfStmt)) => {
                let (mut estmts, ecomp) = self.lower_if(e);
                let ident = astbuild::ident(self.ast, &ecomp, line, col);
                let value = astbuild::static_call(
                    self.ast,
                    self.builder,
                    "buildEither",
                    vec![(Some("second"), ident)],
                    line,
                    col,
                );
                estmts.push(astbuild::assign(self.ast, &comp, value, line, col));
                astbuild::block(self.ast, estmts, line, col)
            }
            _ => {
                // Bare `if`: the not-taken arm contributes `buildOptional(nil)`.
                let nil = astbuild::nil_literal(self.ast, line, col);
                let value = self.build_optional(nil, line, col);
                let assign = astbuild::assign(self.ast, &comp, value, line, col);
                astbuild::block(self.ast, vec![assign], line, col)
            }
        };

        let new_if = self.ast.add(NodeKind::IfStmt, None, line, col);
        for cond in conds {
            self.ast.append_child(new_if, cond);
        }
        self.ast.append_child(new_if, new_then);
        self.ast.append_child(new_if, new_else);

        let var = astbuild::var_decl(self.ast, &comp, line, col);
        (vec![var, new_if], comp)
    }

    /// Lower a `for` to an accumulator-array fold. Returns
    /// `(var $arr = []; for … { …; $arr.append(v) }; let $c = buildArray($arr),
    /// component-name $c)`. The loop stays a real statement, so pattern
    /// bindings, `where`, `break`/`continue`, and labels are all preserved.
    fn lower_for(&mut self, for_node: NodeId) -> (Vec<NodeId>, String) {
        let acc = self.fresh();
        let comp = self.fresh();
        let (line, col) = (
            self.ast.node(for_node).line(),
            self.ast.node(for_node).col(),
        );
        let label = self.ast.node(for_node).text().map(str::to_string);
        let kids = child_ids(self.ast, for_node);
        // The body is the last `Block`; everything before it (pattern, iterable,
        // optional `where`) is preserved verbatim.
        let body_idx = kids
            .iter()
            .rposition(|&c| self.ast.node(c).kind() == NodeKind::Block)
            .expect("for has a body block");
        let preserved = kids[..body_idx].to_vec();
        let body_block = kids[body_idx];

        // Lower the body, then append its folded value to the accumulator.
        let body_stmts = child_ids(self.ast, body_block);
        let (mut bstmts, bvalue) = self.lower_block(&body_stmts, line, col);
        let acc_ref = astbuild::ident(self.ast, &acc, line, col);
        let append = astbuild::method_call(self.ast, acc_ref, "append", vec![bvalue], line, col);
        bstmts.push(append);
        let new_body = astbuild::block(self.ast, bstmts, line, col);

        let new_for = self.ast.add(NodeKind::ForStmt, label.as_deref(), line, col);
        for c in preserved {
            self.ast.append_child(new_for, c);
        }
        self.ast.append_child(new_for, new_body);

        let empty = astbuild::empty_array(self.ast, line, col);
        let acc_decl = astbuild::var_decl_init(self.ast, &acc, empty, line, col);
        let acc_ident = astbuild::ident(self.ast, &acc, line, col);
        let build_array = astbuild::static_call(
            self.ast,
            self.builder,
            "buildArray",
            vec![(None, acc_ident)],
            line,
            col,
        );
        let comp_decl = astbuild::fresh_let(self.ast, &comp, build_array, line, col);
        (vec![acc_decl, new_for, comp_decl], comp)
    }

    /// `Builder.buildOptional(value)` when the builder declares it; otherwise the
    /// value passes through (matching the runtime's structural selection).
    fn build_optional(&mut self, value: NodeId, line: u32, col: u32) -> NodeId {
        if self.methods.has("buildOptional") {
            astbuild::static_call(
                self.ast,
                self.builder,
                "buildOptional",
                vec![(None, value)],
                line,
                col,
            )
        } else {
            value
        }
    }
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

    const COND_BUILDER: &str = "@resultBuilder\nstruct B {\n\
        static func buildExpression(_ v: String) -> String { v }\n\
        static func buildBlock(_ parts: String...) -> String { \"\" }\n\
        static func buildEither(first: String) -> String { first }\n\
        static func buildEither(second: String) -> String { second }\n\
        static func buildOptional(_ part: String?) -> String { part ?? \"\" }\n}\n";

    fn analyzed_cond(extra: &str) -> Ast {
        let mut ast = parse(&format!("{COND_BUILDER}{extra}")).expect("parse ok");
        let diags = analyze(&mut ast);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        ast
    }

    /// The member name of a synthesized `Builder.method(...)` call expr.
    fn call_method<'a>(call: tswift_ast::Node<'a>) -> Option<&'a str> {
        call.children().next().and_then(|callee| callee.text())
    }

    #[test]
    fn if_else_lowers_to_build_either_first_and_second() {
        let ast =
            analyzed_cond("@B\nfunc g(_ f: Bool) -> String {\n if f { \"a\" } else { \"b\" }\n}");
        let body = func_body(&ast, "g");
        // var $c ; if { ... } ; return buildBlock($c)
        let kids: Vec<_> = body.children().map(|c| c.kind()).collect();
        assert_eq!(
            kids,
            vec![NodeKind::VarDecl, NodeKind::IfStmt, NodeKind::ReturnStmt]
        );
        let if_stmt = body.children().nth(1).unwrap();
        let blocks: Vec<_> = if_stmt
            .children()
            .filter(|c| c.kind() == NodeKind::Block)
            .collect();
        // then-branch ends in `$c = B.buildEither(first: ...)`
        let then_assign = blocks[0].children().last().unwrap();
        assert_eq!(then_assign.kind(), NodeKind::AssignExpr);
        let first_call = then_assign.children().nth(1).unwrap();
        assert_eq!(call_method(first_call), Some("buildEither"));
        assert_eq!(
            first_call.children().nth(1).unwrap().arg_label(),
            Some("first")
        );
        // else-branch ends in `$c = B.buildEither(second: ...)`
        let else_assign = blocks[1].children().last().unwrap();
        let second_call = else_assign.children().nth(1).unwrap();
        assert_eq!(
            second_call.children().nth(1).unwrap().arg_label(),
            Some("second")
        );
    }

    #[test]
    fn bare_if_lowers_to_build_optional_with_a_nil_arm() {
        let ast = analyzed_cond("@B\nfunc g(_ f: Bool) -> String {\n if f { \"a\" }\n}");
        let body = func_body(&ast, "g");
        let if_stmt = body.children().nth(1).unwrap();
        let blocks: Vec<_> = if_stmt
            .children()
            .filter(|c| c.kind() == NodeKind::Block)
            .collect();
        assert_eq!(blocks.len(), 2, "a synthesized else arm is added");
        let then_call = blocks[0]
            .children()
            .last()
            .unwrap()
            .children()
            .nth(1)
            .unwrap();
        assert_eq!(call_method(then_call), Some("buildOptional"));
        // The else arm assigns buildOptional(nil).
        let else_call = blocks[1]
            .children()
            .last()
            .unwrap()
            .children()
            .nth(1)
            .unwrap();
        assert_eq!(call_method(else_call), Some("buildOptional"));
        assert_eq!(
            else_call.children().nth(1).unwrap().kind(),
            NodeKind::NilLiteral
        );
    }

    #[test]
    fn if_let_condition_is_preserved() {
        let ast = analyzed_cond(
            "@B\nfunc g(_ m: String?) -> String {\n if let m = m { m } else { \"x\" }\n}",
        );
        let body = func_body(&ast, "g");
        let if_stmt = body.children().nth(1).unwrap();
        // The `if let` binding survives as the first condition child.
        let cond = if_stmt.children().next().unwrap();
        assert_eq!(cond.kind(), NodeKind::LetDecl);
    }

    #[test]
    fn else_if_chain_nests_under_build_either_second() {
        let ast = analyzed_cond(
            "@B\nfunc g(_ n: Int) -> String {\n if n > 1 { \"a\" } else if n > 0 { \"b\" } else { \"c\" }\n}",
        );
        let body = func_body(&ast, "g");
        let outer_if = body.children().nth(1).unwrap();
        let else_block = outer_if
            .children()
            .filter(|c| c.kind() == NodeKind::Block)
            .nth(1)
            .unwrap();
        // The nested else-if lowers to its own `var $c2; if ...` plus the
        // `$c = buildEither(second: $c2)` assignment.
        assert!(else_block.children().any(|c| c.kind() == NodeKind::VarDecl));
        assert!(else_block.children().any(|c| c.kind() == NodeKind::IfStmt));
        let assign = else_block.children().last().unwrap();
        assert_eq!(assign.kind(), NodeKind::AssignExpr);
        let call = assign.children().nth(1).unwrap();
        assert_eq!(call_method(call), Some("buildEither"));
        assert_eq!(call.children().nth(1).unwrap().arg_label(), Some("second"));
    }

    const FOR_BUILDER: &str = "@resultBuilder\nstruct B {\n\
        static func buildExpression(_ v: String) -> String { v }\n\
        static func buildBlock(_ parts: String...) -> String { \"\" }\n\
        static func buildArray(_ parts: [String]) -> String { \"\" }\n}\n";

    #[test]
    fn for_lowers_to_accumulator_and_build_array() {
        let mut ast = parse(&format!(
            "{FOR_BUILDER}@B\nfunc g() -> String {{\n for x in [\"a\"] {{ x }}\n}}"
        ))
        .expect("parse ok");
        let diags = analyze(&mut ast);
        assert!(diags.is_empty(), "{diags:?}");
        let body = func_body(&ast, "g");
        // var $acc = [] ; for ... { ...; $acc.append(...) } ; let $c = buildArray($acc) ; return buildBlock($c)
        let kids: Vec<_> = body.children().map(|c| c.kind()).collect();
        assert_eq!(
            kids,
            vec![
                NodeKind::VarDecl,
                NodeKind::ForStmt,
                NodeKind::LetDecl,
                NodeKind::ReturnStmt
            ]
        );
        // The accumulator var is initialized to an array literal.
        let acc = body.children().next().unwrap();
        assert_eq!(
            acc.children().nth(1).unwrap().kind(),
            NodeKind::ArrayLiteral
        );
        // The rewritten loop body ends in `$acc.append(...)`.
        let for_stmt = body.children().nth(1).unwrap();
        let loop_body = for_stmt
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .unwrap();
        let append = loop_body.children().last().unwrap();
        assert_eq!(append.kind(), NodeKind::CallExpr);
        assert_eq!(
            append.children().next().unwrap().text(),
            Some("append"),
            "loop appends each component to the accumulator"
        );
        // The component is `B.buildArray($acc)`.
        let comp = body.children().nth(2).unwrap();
        let call = comp.children().nth(1).unwrap();
        assert_eq!(call.children().next().unwrap().text(), Some("buildArray"));
    }

    #[test]
    fn for_preserves_pattern_where_and_label() {
        let mut ast = parse(&format!(
            "{FOR_BUILDER}@B\nfunc g() -> String {{\n loop: for (k, v) in xs where k {{ v }}\n}}"
        ))
        .expect("parse ok");
        analyze(&mut ast);
        let body = func_body(&ast, "g");
        let for_stmt = body.children().nth(1).unwrap();
        assert_eq!(for_stmt.text(), Some("loop"), "loop label is preserved");
        // The original tuple pattern and where guard survive as for children.
        assert!(for_stmt
            .children()
            .any(|c| c.kind() == NodeKind::TuplePattern));
        assert!(for_stmt
            .children()
            .any(|c| c.kind() == NodeKind::BinaryExpr || c.kind() == NodeKind::IdentExpr));
    }

    #[test]
    fn for_without_build_array_is_left_for_the_runtime_path() {
        // The builder lacks buildArray, so a `for` body is not handleable.
        let ast = analyzed("@B\nfunc g() -> String {\n for x in [\"a\"] { x }\n}");
        let body = func_body(&ast, "g");
        assert!(body.children().any(|c| c.kind() == NodeKind::ForStmt));
    }

    #[test]
    fn control_flow_body_is_left_for_the_runtime_path() {
        // A `for` is not yet handled, so the body and its attribute are intact.
        let ast = analyzed("@B\nfunc g() -> String {\n \"a\"\n for w in [\"x\"] { w }\n}");
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
        // Body still has the original `for` statement.
        let body = func_body(&ast, "g");
        assert!(body.children().any(|c| c.kind() == NodeKind::ForStmt));
    }
}
