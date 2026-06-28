//! The result-builder transform pass.
//!
//! A `@Builder`-annotated function body is a *result-builder DSL*: a sequence of
//! component expressions the builder folds into one value (SE-0289). This pass
//! performs that fold at compile time, rewriting the body into ordinary
//! `Builder.buildBlock(...)` method calls and **erasing** the builder attribute,
//! so the interpreter sees plain static calls and needs no builder-specific
//! evaluation.
//!
//! `async` builder closures evaluate components in order, but are **gated** on
//! async-closure support elsewhere in the runtime; the rewrite itself is
//! transparent to `async`/`throws` unwinding, so no special handling is needed
//! here once that support lands.
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
        // Validate declarations and bodies *before* the rewrite mutates them.
        let mut diags = Vec::new();
        validate(ast, ast.root(), symbols, &mut diags);

        let mut targets = Vec::new();
        collect_targets(ast, ast.root(), symbols, &mut targets);
        for (decl, builder) in targets {
            transform_func(ast, decl, &builder, symbols, &mut diags);
        }

        // Contextual builders: a closure literal passed to a `@Builder`
        // parameter is transformed as a builder body (#127).
        let mut closure_targets = Vec::new();
        collect_closure_targets(ast, ast.root(), symbols, &mut closure_targets);
        for (closure, builder) in closure_targets {
            transform_closure(ast, closure, &builder, symbols, &mut diags);
        }
        diags
    }
}

/// Find closure-literal arguments passed to `@Builder` parameters, paired with
/// the builder name. Positional match: parameter *i* maps to call child *i+1*
/// (child 0 is the callee), which also covers trailing-closure syntax.
fn collect_closure_targets(
    ast: &Ast,
    parent: NodeId,
    symbols: &Symbols,
    out: &mut Vec<(NodeId, String)>,
) {
    for child in child_ids(ast, parent) {
        if ast.node(child).kind() == NodeKind::CallExpr {
            let kids = child_ids(ast, child);
            if let Some(&callee) = kids.first() {
                if ast.node(callee).kind() == NodeKind::IdentExpr {
                    if let Some(name) = ast.node(callee).text() {
                        for (index, builder) in symbols.func_builder_params(name) {
                            if let Some(&arg) = kids.get(index + 1) {
                                if ast.node(arg).kind() == NodeKind::ClosureExpr {
                                    out.push((arg, builder));
                                }
                            }
                        }
                    }
                }
            }
        }
        collect_closure_targets(ast, child, symbols, out);
    }
}

/// Transform a closure literal's body as a builder block. Parameters and capture
/// list are preserved; the statement children become the lowered build calls.
/// The function parameter keeps its builder attribute so the interpreter still
/// transforms *non-literal* closure arguments (a closure passed by name).
fn transform_closure(
    ast: &mut Ast,
    closure: NodeId,
    builder: &str,
    symbols: &Symbols,
    diags: &mut Vec<Diagnostic>,
) {
    let Some(methods) = symbols.result_builder(builder) else {
        return;
    };
    let has_fold = methods.has("buildBlock")
        || (methods.has_arity("buildPartialBlock", 1) && methods.has_arity("buildPartialBlock", 2));
    if !has_fold {
        return;
    }
    let kids = child_ids(ast, closure);
    let (prelude, stmts): (Vec<NodeId>, Vec<NodeId>) = kids.into_iter().partition(|&c| {
        matches!(
            ast.node(c).kind(),
            NodeKind::Param | NodeKind::ClosureCapture
        )
    });
    // A sole `return` bypasses the builder (SE-0289); also avoids re-wrapping a
    // body the transform already produced.
    if stmts.len() == 1 && ast.node(stmts[0]).kind() == NodeKind::ReturnStmt {
        return;
    }
    // Diagnose any unsupported construct; an invalid body is not lowered (the
    // diagnostic stops the run — there is no runtime fallback).
    if validate_builder_body(ast, &stmts, methods, diags) > 0 {
        return;
    }
    let (line, col) = (ast.node(closure).line(), ast.node(closure).col());
    let mut lowering = Lowering {
        ast,
        builder,
        methods,
        counter: 0,
    };
    let (mut lowered, value) = lowering.lower_block(&stmts, line, col);
    let value = lowering.build_final_result(value, line, col);
    let ret = astbuild::return_stmt(lowering.ast, value, line, col);
    lowered.push(ret);
    let mut new_children = prelude;
    new_children.extend(lowered);
    ast.set_children(closure, new_children);
}

/// The set of recognized result-builder method names.
const BUILD_METHODS: &[&str] = &[
    "buildBlock",
    "buildExpression",
    "buildEither",
    "buildOptional",
    "buildArray",
    "buildPartialBlock",
    "buildFinalResult",
    "buildLimitedAvailability",
];

fn diag(ast: &Ast, node: NodeId, message: String) -> Diagnostic {
    Diagnostic {
        message,
        line: ast.node(node).line(),
        col: ast.node(node).col(),
        severity: crate::Severity::Error,
    }
}

/// Structural validation of result-builder declarations (independent of any
/// lowering construct): builder-type method requirements, build-method
/// signatures, and builder attributes on parameters.
fn validate(ast: &Ast, parent: NodeId, symbols: &Symbols, diags: &mut Vec<Diagnostic>) {
    for child in child_ids(ast, parent) {
        let kind = ast.node(child).kind();
        if matches!(
            kind,
            NodeKind::StructDecl | NodeKind::EnumDecl | NodeKind::ClassDecl | NodeKind::ActorDecl
        ) && ast
            .node(child)
            .children()
            .any(|c| c.kind() == NodeKind::Attribute && c.text() == Some("resultBuilder"))
        {
            validate_builder_type(ast, child, symbols, diags);
        }
        if kind == NodeKind::Param {
            validate_param_builder_attr(ast, child, symbols, diags);
        }
        validate(ast, child, symbols, diags);
    }
}

/// A `@resultBuilder` type must declare a `buildBlock` (or the `buildPartialBlock`
/// pair), and each build method must be `static` with a valid signature.
fn validate_builder_type(ast: &Ast, decl: NodeId, symbols: &Symbols, diags: &mut Vec<Diagnostic>) {
    let name = ast.node(decl).text().unwrap_or("");
    // The interpreter dispatches static build methods only on `struct`/`class`
    // types; an `enum`/`actor` builder would lower to calls it cannot execute,
    // so reject it up front rather than miscompiling.
    if matches!(
        ast.node(decl).kind(),
        NodeKind::EnumDecl | NodeKind::ActorDecl
    ) {
        diags.push(diag(
            ast,
            decl,
            format!("result builder '{name}' must be a 'struct' or 'class' type"),
        ));
    }
    if let Some(methods) = symbols.result_builder(name) {
        let has_fold = methods.has("buildBlock")
            || (methods.has_arity("buildPartialBlock", 1)
                && methods.has_arity("buildPartialBlock", 2));
        if !has_fold {
            diags.push(diag(
                ast,
                decl,
                format!(
                    "result builder '{name}' must provide a static 'buildBlock' method (or the 'buildPartialBlock' pair)"
                ),
            ));
        }
        check_overload_ambiguity(ast, decl, methods, diags);
    }
    // Signature checks on each declared build method.
    for func in builder_member_funcs(ast, decl) {
        let Some(mname) = ast.node(func).text() else {
            continue;
        };
        if !BUILD_METHODS.contains(&mname) {
            continue;
        }
        let is_static = ast.node(func).modifiers().iter().any(|m| m == "static");
        if !is_static {
            diags.push(diag(
                ast,
                func,
                format!("result-builder method '{mname}' must be declared 'static'"),
            ));
        }
        if mname == "buildEither" {
            let first = child_ids(ast, func)
                .into_iter()
                .find(|c| ast.node(*c).kind() == NodeKind::Param);
            let label = first.map(|p| param_effective_label(ast, p));
            if !matches!(label.as_deref(), Some("first") | Some("second")) {
                diags.push(diag(
                    ast,
                    func,
                    "'buildEither' must take a single 'first:' or 'second:' parameter".to_string(),
                ));
            }
        }
    }
}

/// Diagnose build-method overloads the forward-only pipeline cannot resolve
/// (Tier B/C, plan §4.1). Overloads of the same name/arity/first-label are
/// resolvable only when separable by **distinct modelled scalar** parameter
/// types (the interpreter then dispatches by the argument's runtime type).
/// Overloads on unmodelled (user/contextual) types, or with duplicate scalar
/// types, are ambiguous and diagnosed rather than silently miscompiled.
fn check_overload_ambiguity(
    ast: &Ast,
    decl: NodeId,
    methods: &BuilderMethods,
    diags: &mut Vec<Diagnostic>,
) {
    use std::collections::HashMap;
    // `buildEither` / `buildPartialBlock` overloads are discriminated by label
    // and arity by design, not by type, so they are never type-only ambiguous.
    // The parser does not preserve whether a bare `name:` parameter carried a
    // label, so grouping the rest by (name, arity) is the reliable key.
    let mut groups: HashMap<(&str, usize), Vec<Option<&str>>> = HashMap::new();
    for m in methods.methods() {
        if matches!(m.name.as_str(), "buildEither" | "buildPartialBlock") {
            continue;
        }
        groups
            .entry((m.name.as_str(), m.arity))
            .or_default()
            .push(m.first_param_type.as_deref());
    }
    for ((name, _), param_types) in groups {
        if param_types.len() < 2 {
            continue;
        }
        // Resolvable iff every overload has a distinct modelled scalar type.
        let mut seen: Vec<tswift_ast::Type> = Vec::new();
        let resolvable = param_types.iter().all(|t| {
            match t.and_then(|t| crate::parse_type_name(t.trim().trim_end_matches('?'))) {
                Some(scalar) if !seen.contains(&scalar) => {
                    seen.push(scalar);
                    true
                }
                _ => false,
            }
        });
        if !resolvable {
            diags.push(diag(
                ast,
                decl,
                format!(
                    "ambiguous result-builder method '{name}': overloads separable only by type cannot be resolved by the forward-only type checker"
                ),
            ));
        }
    }
}

/// The member `FuncDecl`s of a type, whether direct children or nested in a
/// member `Block`.
fn builder_member_funcs(ast: &Ast, decl: NodeId) -> Vec<NodeId> {
    let mut funcs = Vec::new();
    for c in child_ids(ast, decl) {
        match ast.node(c).kind() {
            NodeKind::FuncDecl => funcs.push(c),
            NodeKind::Block => funcs.extend(builder_member_funcs(ast, c)),
            _ => {}
        }
    }
    funcs
}

/// A parameter's effective argument label: its explicit label, or its name when
/// none is written (Swift's `name:` rule).
fn param_effective_label(ast: &Ast, param: NodeId) -> String {
    ast.node(param)
        .arg_label()
        .or_else(|| ast.node(param).text())
        .unwrap_or("")
        .to_string()
}

/// A builder attribute on a parameter is only valid when the parameter has a
/// function type (`() -> T`).
fn validate_param_builder_attr(
    ast: &Ast,
    param: NodeId,
    symbols: &Symbols,
    diags: &mut Vec<Diagnostic>,
) {
    let has_builder_attr = ast
        .node(param)
        .children()
        .filter(|c| c.kind() == NodeKind::Attribute)
        .filter_map(|c| c.text())
        .any(|name| symbols.result_builder(name).is_some());
    if !has_builder_attr {
        return;
    }
    let is_function_typed = child_ids(ast, param).into_iter().any(|c| {
        ast.node(c).kind() == NodeKind::TypeRef
            && ast.node(c).text().is_some_and(|t| t.contains("->"))
    });
    if !is_function_typed {
        diags.push(diag(
            ast,
            param,
            "result-builder attribute can only be applied to a parameter of function type"
                .to_string(),
        ));
    }
}

/// Validate a builder body's statements, emitting a diagnostic for anything the
/// transform cannot lower. Since the runtime fallback is gone, every unsupported
/// construct must surface as an error here rather than silently evaluating as an
/// ordinary body. Returns the number of diagnostics added for this body, so the
/// caller can skip lowering an invalid body.
fn validate_builder_body(
    ast: &Ast,
    stmts: &[NodeId],
    methods: &BuilderMethods,
    diags: &mut Vec<Diagnostic>,
) -> usize {
    let before = diags.len();
    // An explicit `return` is only legal as the sole statement of the body.
    let returns: Vec<NodeId> = stmts
        .iter()
        .copied()
        .filter(|&s| ast.node(s).kind() == NodeKind::ReturnStmt)
        .collect();
    if !returns.is_empty() && stmts.len() > 1 {
        diags.push(diag(
            ast,
            returns[0],
            "cannot use an explicit 'return' statement mixed with other components in a result-builder body"
                .to_string(),
        ));
    }
    for &stmt in stmts {
        validate_builder_stmt(ast, stmt, methods, diags);
    }
    diags.len() - before
}

/// Validate one builder-body statement and (recursively) its nested blocks.
fn validate_builder_stmt(
    ast: &Ast,
    stmt: NodeId,
    methods: &BuilderMethods,
    diags: &mut Vec<Diagnostic>,
) {
    match ast.node(stmt).kind() {
        // Components and pass-through statements the lowering handles directly.
        NodeKind::ExprStmt
        | NodeKind::ReturnStmt
        | NodeKind::GuardStmt
        | NodeKind::DeferStmt
        | NodeKind::LetDecl
        | NodeKind::VarDecl
        | NodeKind::FuncDecl
        | NodeKind::StructDecl
        | NodeKind::EnumDecl
        | NodeKind::ClassDecl
        | NodeKind::ActorDecl
        | NodeKind::ProtocolDecl
        | NodeKind::TypeAliasDecl => {}
        NodeKind::IfStmt => {
            let has_else = if_has_else(ast, stmt);
            if has_else && !methods.has("buildEither") {
                diags.push(diag(
                    ast,
                    stmt,
                    "'if'/'else' in a result-builder body requires the builder to declare 'buildEither(first:)' and 'buildEither(second:)'".to_string(),
                ));
            }
            if !has_else && !methods.has("buildOptional") {
                diags.push(diag(
                    ast,
                    stmt,
                    "'if' without 'else' in a result-builder body requires the builder to declare 'buildOptional'".to_string(),
                ));
            }
            for c in child_ids(ast, stmt) {
                match ast.node(c).kind() {
                    NodeKind::Block => {
                        validate_builder_body(ast, &child_ids(ast, c), methods, diags);
                    }
                    NodeKind::IfStmt => validate_builder_stmt(ast, c, methods, diags),
                    _ => {}
                }
            }
        }
        NodeKind::ForStmt => {
            if !methods.has("buildArray") {
                diags.push(diag(
                    ast,
                    stmt,
                    "'for' in a result-builder body requires the builder to declare 'buildArray'"
                        .to_string(),
                ));
            }
            for c in child_ids(ast, stmt) {
                if ast.node(c).kind() == NodeKind::Block {
                    validate_builder_body(ast, &child_ids(ast, c), methods, diags);
                }
            }
        }
        NodeKind::SwitchStmt => {
            if !methods.has("buildEither") {
                diags.push(diag(
                    ast,
                    stmt,
                    "'switch' in a result-builder body requires the builder to declare 'buildEither'"
                        .to_string(),
                ));
            }
            for clause in child_ids(ast, stmt) {
                if ast.node(clause).kind() == NodeKind::CaseClause {
                    for c in child_ids(ast, clause) {
                        if ast.node(c).kind() == NodeKind::Block {
                            validate_builder_body(ast, &child_ids(ast, c), methods, diags);
                        }
                    }
                }
            }
        }
        NodeKind::WhileStmt | NodeKind::RepeatStmt => {
            diags.push(diag(
                ast,
                stmt,
                "'while'/'repeat' loops are not supported in a result-builder body".to_string(),
            ));
        }
        other => {
            diags.push(diag(
                ast,
                stmt,
                format!(
                    "'{}' is not supported in a result-builder body",
                    other.name()
                ),
            ));
        }
    }
}

/// Whether an `if` has an `else` arm (a second `Block`, or an `else if`).
fn if_has_else(ast: &Ast, if_stmt: NodeId) -> bool {
    let kids = child_ids(ast, if_stmt);
    let Some(then_idx) = kids
        .iter()
        .position(|&c| ast.node(c).kind() == NodeKind::Block)
    else {
        return false;
    };
    kids.get(then_idx + 1).is_some()
}

/// Find every `FuncDecl` carrying a result-builder attribute, paired with the
/// builder type's name. Recurses into nested scopes so builder *methods* (funcs
/// declared inside a type) are found too.
fn collect_targets(ast: &Ast, parent: NodeId, symbols: &Symbols, out: &mut Vec<(NodeId, String)>) {
    for child in child_ids(ast, parent) {
        if is_builder_target(ast.node(child).kind()) {
            if let Some(builder) = builder_attr(ast, child, symbols) {
                out.push((child, builder));
            }
        }
        collect_targets(ast, child, symbols, out);
    }
}

/// Declaration kinds a builder attribute can apply to: a function body, a
/// computed-property getter, or a subscript getter.
fn is_builder_target(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::FuncDecl | NodeKind::VarDecl | NodeKind::SubscriptDecl
    )
}

/// The body block a builder attribute transforms on `decl`: a function's direct
/// block, or the `get` accessor's block of a computed property / subscript.
fn decl_body_block(ast: &Ast, decl: NodeId) -> Option<NodeId> {
    match ast.node(decl).kind() {
        NodeKind::FuncDecl => child_ids(ast, decl)
            .into_iter()
            .find(|c| ast.node(*c).kind() == NodeKind::Block),
        NodeKind::VarDecl | NodeKind::SubscriptDecl => {
            let getter = child_ids(ast, decl).into_iter().find(|c| {
                ast.node(*c).kind() == NodeKind::Accessor && ast.node(*c).text() == Some("get")
            })?;
            child_ids(ast, getter)
                .into_iter()
                .find(|c| ast.node(*c).kind() == NodeKind::Block)
        }
        _ => None,
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

/// Transform `decl`'s body if it is fully handleable; otherwise leave it (and
/// its attribute) untouched for the legacy runtime transform. Applies to
/// `@Builder func`, computed-property getters, and subscript getters alike.
fn transform_func(
    ast: &mut Ast,
    decl: NodeId,
    builder: &str,
    symbols: &Symbols,
    diags: &mut Vec<Diagnostic>,
) {
    let Some(methods) = symbols.result_builder(builder) else {
        return;
    };
    // The builder must declare a fold method: variadic `buildBlock` or the
    // `buildPartialBlock` pair.
    let has_fold = methods.has("buildBlock")
        || (methods.has_arity("buildPartialBlock", 1) && methods.has_arity("buildPartialBlock", 2));
    if !has_fold {
        return;
    }
    let Some(body) = decl_body_block(ast, decl) else {
        return;
    };
    let stmts = child_ids(ast, body);
    // SE-0289: a sole `return` is the result — the body bypasses the builder
    // entirely. Erase the attribute and leave the `return` as ordinary code.
    if stmts.len() == 1 && ast.node(stmts[0]).kind() == NodeKind::ReturnStmt {
        erase_attribute(ast, decl, builder);
        return;
    }
    // Diagnose any unsupported construct; an invalid body is not lowered (the
    // diagnostic stops the run — there is no runtime fallback).
    if validate_builder_body(ast, &stmts, methods, diags) > 0 {
        return;
    }

    let (line, col) = (ast.node(body).line(), ast.node(body).col());
    let mut lowering = Lowering {
        ast,
        builder,
        methods,
        counter: 0,
    };
    let (mut new_body, value) = lowering.lower_block(&stmts, line, col);
    // `buildFinalResult` wraps the outermost block's value (the accessor's
    // result) exactly once, when the builder declares it.
    let value = lowering.build_final_result(value, line, col);
    let ret = astbuild::return_stmt(lowering.ast, value, line, col);
    new_body.push(ret);
    ast.set_children(body, new_body);
    erase_attribute(ast, decl, builder);
}

/// Whether an `if`'s conditions include an availability check
/// (`#available` / `#unavailable`), whose taken branch is wrapped in
/// `buildLimitedAvailability`.
fn is_availability_condition(ast: &Ast, conds: &[NodeId]) -> bool {
    conds.iter().any(|&c| {
        ast.node(c).kind() == NodeKind::CompilerDirective
            && matches!(
                ast.node(c).text(),
                Some("#available") | Some("#unavailable")
            )
    })
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
                NodeKind::SwitchStmt => {
                    let (stmts, name) = self.lower_switch(stmt);
                    out.extend(stmts);
                    components.push(name);
                }
                // A declaration is not a component: leave it in place.
                _ => out.push(stmt),
            }
        }
        let value = self.fold_components(&components, line, col);
        (out, value)
    }

    /// Fold a block's component variables into its single value. Prefers the
    /// `buildPartialBlock` left-fold when the builder declares both halves of
    /// the pair (SE-0289 precedence); otherwise uses variadic `buildBlock`. An
    /// empty block is `buildBlock()`.
    fn fold_components(&mut self, components: &[String], line: u32, col: u32) -> NodeId {
        if !components.is_empty() && self.uses_partial_block() {
            // buildPartialBlock(first: c0), then
            // buildPartialBlock(accumulated: acc, next: cN) left-to-right.
            let first = astbuild::ident(self.ast, &components[0], line, col);
            let mut acc = astbuild::static_call(
                self.ast,
                self.builder,
                "buildPartialBlock",
                vec![(Some("first"), first)],
                line,
                col,
            );
            for name in &components[1..] {
                let next = astbuild::ident(self.ast, name, line, col);
                acc = astbuild::static_call(
                    self.ast,
                    self.builder,
                    "buildPartialBlock",
                    vec![(Some("accumulated"), acc), (Some("next"), next)],
                    line,
                    col,
                );
            }
            return acc;
        }
        let args: Vec<(Option<&str>, NodeId)> = components
            .iter()
            .map(|name| (None, astbuild::ident(self.ast, name, line, col)))
            .collect();
        astbuild::static_call(self.ast, self.builder, "buildBlock", args, line, col)
    }

    /// Whether the builder declares the full `buildPartialBlock` pair
    /// (`first:` arity 1 and `accumulated:next:` arity 2).
    fn uses_partial_block(&self) -> bool {
        self.methods.has_arity("buildPartialBlock", 1)
            && self.methods.has_arity("buildPartialBlock", 2)
    }

    /// `Builder.buildFinalResult(value)` when declared; otherwise `value`.
    fn build_final_result(&mut self, value: NodeId, line: u32, col: u32) -> NodeId {
        if self.methods.has("buildFinalResult") {
            astbuild::static_call(
                self.ast,
                self.builder,
                "buildFinalResult",
                vec![(None, value)],
                line,
                col,
            )
        } else {
            value
        }
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
        // `if #available(…)`: wrap the availability branch's value in
        // buildLimitedAvailability before the surrounding optional/either.
        let tvalue = if is_availability_condition(self.ast, &conds) {
            self.build_limited_availability(tvalue, line, col)
        } else {
            tvalue
        };
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

    /// Lower a `switch` to a fresh component var assigned in every case, wrapping
    /// each case's value in a balanced `buildEither(first:)`/`(second:)` tree by
    /// its position. The `switch` stays a real statement — patterns, `where`
    /// guards, and bindings are preserved; `default` is the last case, so it
    /// lands on the final `second`.
    fn lower_switch(&mut self, switch_node: NodeId) -> (Vec<NodeId>, String) {
        let comp = self.fresh();
        let (line, col) = (
            self.ast.node(switch_node).line(),
            self.ast.node(switch_node).col(),
        );
        let label = self.ast.node(switch_node).text().map(str::to_string);
        let kids = child_ids(self.ast, switch_node);
        let subject = kids[0];
        let clauses = &kids[1..];
        let count = clauses.len();

        let new_switch = self
            .ast
            .add(NodeKind::SwitchStmt, label.as_deref(), line, col);
        self.ast.append_child(new_switch, subject);
        for (idx, &clause) in clauses.iter().enumerate() {
            let ckids = child_ids(self.ast, clause);
            let body_idx = ckids
                .iter()
                .rposition(|&c| self.ast.node(c).kind() == NodeKind::Block)
                .expect("case has a body block");
            let preserved = ckids[..body_idx].to_vec();
            let body_block = ckids[body_idx];

            let body_stmts = child_ids(self.ast, body_block);
            let (mut bstmts, bvalue) = self.lower_block(&body_stmts, line, col);
            let injected = self.inject_either(bvalue, 0, count, idx, line, col);
            bstmts.push(astbuild::assign(self.ast, &comp, injected, line, col));
            let new_body = astbuild::block(self.ast, bstmts, line, col);

            let clause_text = self.ast.node(clause).text().map(str::to_string);
            let new_clause = self
                .ast
                .add(NodeKind::CaseClause, clause_text.as_deref(), line, col);
            for p in preserved {
                self.ast.append_child(new_clause, p);
            }
            self.ast.append_child(new_clause, new_body);
            self.ast.append_child(new_switch, new_clause);
        }

        let var = astbuild::var_decl(self.ast, &comp, line, col);
        (vec![var, new_switch], comp)
    }

    /// Wrap `value` for case `idx` of `count` in a balanced buildEither tree:
    /// cases in the lower half take `buildEither(first:)`, the upper half
    /// `buildEither(second:)`, recursively. A singleton leaf is the value
    /// itself.
    fn inject_either(
        &mut self,
        value: NodeId,
        lo: usize,
        hi: usize,
        idx: usize,
        line: u32,
        col: u32,
    ) -> NodeId {
        if hi - lo <= 1 {
            return value;
        }
        let mid = lo + (hi - lo).div_ceil(2);
        let (label, inner) = if idx < mid {
            ("first", self.inject_either(value, lo, mid, idx, line, col))
        } else {
            ("second", self.inject_either(value, mid, hi, idx, line, col))
        };
        astbuild::static_call(
            self.ast,
            self.builder,
            "buildEither",
            vec![(Some(label), inner)],
            line,
            col,
        )
    }

    /// `Builder.buildLimitedAvailability(value)` when the builder declares it;
    /// otherwise the value passes through.
    fn build_limited_availability(&mut self, value: NodeId, line: u32, col: u32) -> NodeId {
        if self.methods.has("buildLimitedAvailability") {
            astbuild::static_call(
                self.ast,
                self.builder,
                "buildLimitedAvailability",
                vec![(None, value)],
                line,
                col,
            )
        } else {
            value
        }
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
    fn for_without_build_array_is_diagnosed() {
        // The builder lacks buildArray: a `for` body can no longer be lowered
        // (the runtime fallback is gone), so it must be diagnosed.
        let diags = diags_of(&format!(
            "{STRING_BUILDER}@B\nfunc g() -> String {{\n for x in [\"a\"] {{ x }}\n}}"
        ));
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("'for'") && d.message.contains("buildArray")),
            "{diags:?}"
        );
    }

    #[test]
    fn partial_block_pair_folds_left_to_right() {
        let src = "@resultBuilder\nstruct P {\n\
            static func buildExpression(_ v: String) -> String { v }\n\
            static func buildPartialBlock(first: String) -> String { first }\n\
            static func buildPartialBlock(accumulated: String, next: String) -> String { accumulated }\n}\n\
            @P\nfunc g() -> String {\n \"a\"\n \"b\"\n \"c\"\n}";
        let mut ast = parse(src).expect("parse ok");
        analyze(&mut ast);
        let body = func_body(&ast, "g");
        let ret_call = body.children().last().unwrap().children().next().unwrap();
        // Outermost call is buildPartialBlock(accumulated:next:).
        assert_eq!(
            ret_call.children().next().unwrap().text(),
            Some("buildPartialBlock")
        );
        let labels: Vec<_> = ret_call.children().skip(1).map(|c| c.arg_label()).collect();
        assert_eq!(labels, vec![Some("accumulated"), Some("next")]);
        // Its accumulated arg is itself a buildPartialBlock call (the fold).
        let inner = ret_call.children().nth(1).unwrap();
        assert_eq!(
            inner.children().next().unwrap().text(),
            Some("buildPartialBlock")
        );
    }

    #[test]
    fn partial_block_is_preferred_when_build_block_also_present() {
        let src = "@resultBuilder\nstruct P {\n\
            static func buildBlock(_ parts: String...) -> String { \"\" }\n\
            static func buildPartialBlock(first: String) -> String { first }\n\
            static func buildPartialBlock(accumulated: String, next: String) -> String { accumulated }\n}\n\
            @P\nfunc g() -> String {\n \"a\"\n \"b\"\n}";
        let mut ast = parse(src).expect("parse ok");
        analyze(&mut ast);
        let body = func_body(&ast, "g");
        let ret_call = body.children().last().unwrap().children().next().unwrap();
        assert_eq!(
            ret_call.children().next().unwrap().text(),
            Some("buildPartialBlock"),
            "partial-block is preferred over variadic buildBlock"
        );
    }

    #[test]
    fn build_final_result_wraps_the_outermost_value_once() {
        let src = "@resultBuilder\nstruct F {\n\
            static func buildBlock(_ parts: String...) -> String { \"\" }\n\
            static func buildFinalResult(_ v: String) -> String { v }\n}\n\
            @F\nfunc g() -> String {\n \"a\"\n}";
        let mut ast = parse(src).expect("parse ok");
        analyze(&mut ast);
        let body = func_body(&ast, "g");
        let ret_call = body.children().last().unwrap().children().next().unwrap();
        assert_eq!(
            ret_call.children().next().unwrap().text(),
            Some("buildFinalResult")
        );
        // Its sole arg is the buildBlock value (wrapped exactly once).
        let inner = ret_call.children().nth(1).unwrap();
        assert_eq!(inner.children().next().unwrap().text(), Some("buildBlock"));
    }

    #[test]
    fn switch_lowers_to_a_balanced_build_either_tree() {
        // 3 cases: first(first(c0)), first(second(c1)), second(c2/default).
        let mut ast = parse(&format!(
            "{COND_BUILDER}@B\nfunc g(_ n: Int) -> String {{\n\
             switch n {{\n case 0: \"a\"\n case 1: \"b\"\n default: \"c\" }}\n}}"
        ))
        .expect("parse ok");
        let diags = analyze(&mut ast);
        assert!(diags.is_empty(), "{diags:?}");
        let body = func_body(&ast, "g");
        let kids: Vec<_> = body.children().map(|c| c.kind()).collect();
        assert_eq!(
            kids,
            vec![
                NodeKind::VarDecl,
                NodeKind::SwitchStmt,
                NodeKind::ReturnStmt
            ]
        );
        let switch = body.children().nth(1).unwrap();
        let clauses: Vec<_> = switch
            .children()
            .filter(|c| c.kind() == NodeKind::CaseClause)
            .collect();
        assert_eq!(clauses.len(), 3);

        // The default clause (last) assigns buildEither(second: c2) — one level.
        let default_clause = clauses[2];
        assert_eq!(default_clause.text(), Some("default"));
        let assign = default_clause
            .children()
            .last()
            .unwrap()
            .children()
            .last()
            .unwrap();
        assert_eq!(assign.kind(), NodeKind::AssignExpr);
        let call = assign.children().nth(1).unwrap();
        assert_eq!(call.children().next().unwrap().text(), Some("buildEither"));
        assert_eq!(call.children().nth(1).unwrap().arg_label(), Some("second"));
        // Case 0 nests two levels: first(first(c0)).
        let case0_call = clauses[0]
            .children()
            .last()
            .unwrap()
            .children()
            .last()
            .unwrap()
            .children()
            .nth(1)
            .unwrap();
        assert_eq!(
            case0_call.children().nth(1).unwrap().arg_label(),
            Some("first")
        );
        let inner = case0_call.children().nth(1).unwrap();
        assert_eq!(inner.children().next().unwrap().text(), Some("buildEither"));
    }

    #[test]
    fn switch_preserves_patterns_and_where_guards() {
        let mut ast = parse(&format!(
            "{COND_BUILDER}@B\nfunc g(_ n: Int) -> String {{\n\
             switch n {{\n case let x where x > 0: \"a\"\n default: \"b\" }}\n}}"
        ))
        .expect("parse ok");
        analyze(&mut ast);
        let body = func_body(&ast, "g");
        let switch = body.children().nth(1).unwrap();
        let first_case = switch
            .children()
            .find(|c| c.kind() == NodeKind::CaseClause)
            .unwrap();
        assert!(first_case
            .children()
            .any(|c| c.kind() == NodeKind::WhereClause));
        assert!(first_case
            .children()
            .any(|c| c.kind() == NodeKind::NamePattern));
    }

    const AVAIL_BUILDER: &str = "@resultBuilder\nstruct B {\n\
        static func buildExpression(_ v: String) -> String { v }\n\
        static func buildBlock(_ parts: String...) -> String { \"\" }\n\
        static func buildEither(first: String) -> String { first }\n\
        static func buildEither(second: String) -> String { second }\n\
        static func buildOptional(_ part: String?) -> String { part ?? \"\" }\n\
        static func buildLimitedAvailability(_ v: String) -> String { v }\n}\n";

    #[test]
    fn bare_if_available_wraps_in_limited_availability_then_optional() {
        let mut ast = parse(&format!(
            "{AVAIL_BUILDER}@B\nfunc g() -> String {{\n if #available(iOS 13, *) {{ \"a\" }}\n}}"
        ))
        .expect("parse ok");
        let diags = analyze(&mut ast);
        assert!(diags.is_empty(), "{diags:?}");
        let body = func_body(&ast, "g");
        let if_stmt = body.children().nth(1).unwrap();
        let then_block = if_stmt
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .unwrap();
        // then assigns buildOptional(buildLimitedAvailability(...))
        let optional_call = then_block
            .children()
            .last()
            .unwrap()
            .children()
            .nth(1)
            .unwrap();
        assert_eq!(
            optional_call.children().next().unwrap().text(),
            Some("buildOptional")
        );
        let inner = optional_call.children().nth(1).unwrap();
        assert_eq!(
            inner.children().next().unwrap().text(),
            Some("buildLimitedAvailability")
        );
    }

    #[test]
    fn if_available_with_else_wraps_only_the_availability_branch() {
        let mut ast = parse(&format!(
            "{AVAIL_BUILDER}@B\nfunc g() -> String {{\n if #available(iOS 13, *) {{ \"a\" }} else {{ \"b\" }}\n}}"
        ))
        .expect("parse ok");
        analyze(&mut ast);
        let body = func_body(&ast, "g");
        let if_stmt = body.children().nth(1).unwrap();
        let blocks: Vec<_> = if_stmt
            .children()
            .filter(|c| c.kind() == NodeKind::Block)
            .collect();
        // then: buildEither(first: buildLimitedAvailability(...))
        let first_call = blocks[0]
            .children()
            .last()
            .unwrap()
            .children()
            .nth(1)
            .unwrap();
        assert_eq!(
            first_call
                .children()
                .nth(1)
                .unwrap()
                .children()
                .next()
                .unwrap()
                .text(),
            Some("buildLimitedAvailability")
        );
        // else: buildEither(second: ...) with NO limited-availability wrap
        let second_call = blocks[1]
            .children()
            .last()
            .unwrap()
            .children()
            .nth(1)
            .unwrap();
        let second_arg = second_call.children().nth(1).unwrap();
        assert_ne!(
            second_arg.children().next().and_then(|c| c.text()),
            Some("buildLimitedAvailability")
        );
    }

    /// Locate a var/subscript decl by checking it carries no attribute and has
    /// a transformed `get` accessor block.
    fn getter_block<'a>(ast: &'a Ast, name_kind: NodeKind) -> tswift_ast::Node<'a> {
        fn find(ast: &Ast, parent: NodeId, kind: NodeKind) -> Option<NodeId> {
            for c in child_ids(ast, parent) {
                if ast.node(c).kind() == kind {
                    return Some(c);
                }
                if let Some(f) = find(ast, c, kind) {
                    return Some(f);
                }
            }
            None
        }
        let decl = find(ast, ast.root(), name_kind).expect("decl present");
        let getter = ast
            .node(decl)
            .children()
            .find(|c| c.kind() == NodeKind::Accessor)
            .expect("get accessor");
        getter
            .children()
            .find(|c| c.kind() == NodeKind::Block)
            .expect("getter block")
    }

    #[test]
    fn computed_property_getter_is_transformed() {
        let ast = analyzed("struct S {\n @B var body: String {\n \"a\"\n \"b\"\n }\n}");
        let block = getter_block(&ast, NodeKind::VarDecl);
        let kids: Vec<_> = block.children().map(|c| c.kind()).collect();
        assert_eq!(
            kids,
            vec![NodeKind::LetDecl, NodeKind::LetDecl, NodeKind::ReturnStmt]
        );
        // The builder attribute is erased from the property decl.
        fn find_var(ast: &Ast, parent: NodeId) -> Option<NodeId> {
            for c in child_ids(ast, parent) {
                if ast.node(c).kind() == NodeKind::VarDecl {
                    return Some(c);
                }
                if let Some(f) = find_var(ast, c) {
                    return Some(f);
                }
            }
            None
        }
        let var = find_var(&ast, ast.root()).unwrap();
        assert!(!ast
            .node(var)
            .children()
            .any(|c| c.kind() == NodeKind::Attribute));
    }

    #[test]
    fn subscript_getter_is_transformed() {
        let ast = analyzed(
            "struct S {\n @B subscript(_ i: String) -> String {\n get {\n \"a\"\n i\n }\n }\n}",
        );
        let block = getter_block(&ast, NodeKind::SubscriptDecl);
        assert_eq!(
            block.children().last().unwrap().kind(),
            NodeKind::ReturnStmt
        );
        let ret_call = block.children().last().unwrap().children().next().unwrap();
        assert_eq!(
            ret_call.children().next().unwrap().text(),
            Some("buildBlock")
        );
    }

    #[test]
    fn nested_result_builder_is_recognized() {
        // A @resultBuilder declared inside a struct is collected by Symbols and
        // its attribute drives the transform of a method in the same scope.
        let src = "struct Outer {\n\
            @resultBuilder\n struct Inner {\n\
              static func buildExpression(_ v: String) -> String { v }\n\
              static func buildBlock(_ parts: String...) -> String { \"\" }\n }\n\
            @Inner\n func make() -> String {\n \"a\"\n }\n}";
        let mut ast = parse(src).expect("parse ok");
        analyze(&mut ast);
        let body = func_body(&ast, "make");
        // Transformed: ends in `return Inner.buildBlock(...)`.
        let ret_call = body.children().last().unwrap().children().next().unwrap();
        let callee = ret_call.children().next().unwrap();
        assert_eq!(callee.text(), Some("buildBlock"));
        assert_eq!(callee.children().next().unwrap().text(), Some("Inner"));
    }

    fn diags_of(src: &str) -> Vec<Diagnostic> {
        let mut ast = parse(src).expect("parse ok");
        analyze(&mut ast)
    }

    #[test]
    fn builder_attr_on_non_function_param_is_diagnosed() {
        let diags = diags_of(
            "@resultBuilder\nstruct SB {\n static func buildBlock(_ p: String...) -> String { \"\" } }\n\
             func wrap(@SB _ c: Int) -> String { \"x\" }",
        );
        assert!(
            diags.iter().any(|d| d.message.contains("function type")),
            "{diags:?}"
        );
    }

    #[test]
    fn function_typed_builder_param_is_accepted() {
        let diags = diags_of(
            "@resultBuilder\nstruct SB {\n static func buildBlock(_ p: String...) -> String { \"\" } }\n\
             func wrap(@SB _ c: () -> String) -> String { c() }",
        );
        assert!(
            !diags.iter().any(|d| d.message.contains("function type")),
            "{diags:?}"
        );
    }

    #[test]
    fn builder_missing_build_block_is_diagnosed() {
        let diags =
            diags_of("@resultBuilder\nstruct Bad {\n static func buildExpression(_ v: String) -> String { v } }");
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("must provide a static 'buildBlock'")),
            "{diags:?}"
        );
    }

    #[test]
    fn non_static_build_method_is_diagnosed() {
        let diags = diags_of(
            "@resultBuilder\nstruct SB {\n func buildBlock(_ p: String...) -> String { \"\" } }",
        );
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("must be declared 'static'")),
            "{diags:?}"
        );
    }

    #[test]
    fn build_either_without_first_or_second_is_diagnosed() {
        let diags = diags_of(
            "@resultBuilder\nstruct SB {\n\
             static func buildBlock(_ p: String...) -> String { \"\" }\n\
             static func buildEither(_ v: String) -> String { v } }",
        );
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("'first:' or 'second:'")),
            "{diags:?}"
        );
    }

    #[test]
    fn while_loop_in_builder_body_is_diagnosed() {
        let diags = diags_of(
            "@resultBuilder\nstruct SB {\n static func buildBlock(_ p: String...) -> String { \"\" } }\n\
             @SB\nfunc g() -> String {\n while true { \"x\" }\n }",
        );
        assert!(
            diags.iter().any(|d| d.message.contains("'while'/'repeat'")),
            "{diags:?}"
        );
    }

    #[test]
    fn return_mixed_with_components_is_diagnosed() {
        let diags = diags_of(
            "@resultBuilder\nstruct SB {\n static func buildBlock(_ p: String...) -> String { \"\" } }\n\
             @SB\nfunc g() -> String {\n \"a\"\n return \"b\"\n }",
        );
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("explicit 'return'")),
            "{diags:?}"
        );
    }

    #[test]
    fn sole_return_in_builder_body_is_allowed() {
        let diags = diags_of(
            "@resultBuilder\nstruct SB {\n static func buildBlock(_ p: String...) -> String { \"\" } }\n\
             @SB\nfunc g() -> String {\n return \"b\"\n }",
        );
        assert!(
            !diags
                .iter()
                .any(|d| d.message.contains("explicit 'return'")),
            "{diags:?}"
        );
    }

    #[test]
    fn distinct_scalar_build_expression_overloads_are_not_ambiguous() {
        let diags = diags_of(
            "@resultBuilder\nstruct B {\n\
             static func buildExpression(_ v: String) -> String { v }\n\
             static func buildExpression(_ v: Int) -> String { \"\" }\n\
             static func buildBlock(_ p: String...) -> String { \"\" } }",
        );
        assert!(
            !diags.iter().any(|d| d.message.contains("ambiguous")),
            "{diags:?}"
        );
    }

    #[test]
    fn type_only_overloads_on_unmodelled_types_are_ambiguous() {
        let diags = diags_of(
            "struct Foo {}\nstruct Bar {}\n@resultBuilder\nstruct B {\n\
             static func buildExpression(_ v: Foo) -> String { \"\" }\n\
             static func buildExpression(_ v: Bar) -> String { \"\" }\n\
             static func buildBlock(_ p: String...) -> String { \"\" } }",
        );
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("ambiguous result-builder method")),
            "{diags:?}"
        );
    }

    #[test]
    fn build_either_overloads_are_not_flagged_ambiguous() {
        let diags = diags_of(
            "@resultBuilder\nstruct B {\n\
             static func buildBlock(_ p: String...) -> String { \"\" }\n\
             static func buildEither(first: String) -> String { first }\n\
             static func buildEither(second: String) -> String { second } }",
        );
        assert!(
            !diags.iter().any(|d| d.message.contains("ambiguous")),
            "{diags:?}"
        );
    }

    /// Find the first `ClosureExpr` in the tree.
    fn first_closure(ast: &Ast) -> tswift_ast::Node<'_> {
        fn find(ast: &Ast, parent: NodeId) -> Option<NodeId> {
            for c in child_ids(ast, parent) {
                if ast.node(c).kind() == NodeKind::ClosureExpr {
                    return Some(c);
                }
                if let Some(f) = find(ast, c) {
                    return Some(f);
                }
            }
            None
        }
        ast.node(find(ast, ast.root()).expect("closure present"))
    }

    #[test]
    fn closure_literal_argument_to_builder_param_is_transformed() {
        let ast = analyzed(
            "func wrap(@B _ content: () -> String) -> String { content() }\n\
             let r = wrap { \"one\"\n \"two\" }",
        );
        let closure = first_closure(&ast);
        // Body rewritten to two lets + a `return B.buildBlock(...)`.
        let kids: Vec<_> = closure.children().map(|c| c.kind()).collect();
        assert_eq!(
            kids,
            vec![NodeKind::LetDecl, NodeKind::LetDecl, NodeKind::ReturnStmt]
        );
        let ret_call = closure
            .children()
            .last()
            .unwrap()
            .children()
            .next()
            .unwrap();
        assert_eq!(
            ret_call.children().next().unwrap().text(),
            Some("buildBlock")
        );
    }

    #[test]
    fn closure_with_params_keeps_them_and_transforms_body() {
        let ast = analyzed(
            "func wrap(@B _ content: () -> String) -> String { content() }\n\
             let r = wrap { \"x\" }",
        );
        let closure = first_closure(&ast);
        // Single expression -> one let + return.
        assert_eq!(
            closure.children().last().unwrap().kind(),
            NodeKind::ReturnStmt
        );
    }

    #[test]
    fn non_closure_argument_to_builder_param_is_untouched() {
        // Passing a closure by name is left for the runtime transform.
        let ast = analyzed(
            "func wrap(@B _ content: () -> String) -> String { content() }\n\
             let p = { \"a\"\n \"b\" }\n let r = wrap(p)",
        );
        // The `p` closure (assigned to a let, not a literal arg) is NOT rewritten.
        let closure = first_closure(&ast);
        let kinds: Vec<_> = closure.children().map(|c| c.kind()).collect();
        assert_eq!(kinds, vec![NodeKind::ExprStmt, NodeKind::ExprStmt]);
    }

    #[test]
    fn sole_return_bypasses_the_builder() {
        // The body is left as a plain `return`, and the attribute is erased.
        let ast = analyzed("@B\nfunc g() -> String {\n return \"x\"\n}");
        let body = func_body(&ast, "g");
        let kids: Vec<_> = body.children().map(|c| c.kind()).collect();
        assert_eq!(kids, vec![NodeKind::ReturnStmt]);
        // No synthesized build calls: the return value is the literal itself.
        let ret = body.children().next().unwrap();
        assert_eq!(
            ret.children().next().unwrap().kind(),
            NodeKind::StringLiteral
        );
    }

    #[test]
    fn guard_is_passed_through_as_control_flow() {
        let ast = analyzed(
            "@B\nfunc g(_ x: Int) -> String {\n guard x > 0 else { return \"n\" }\n \"a\"\n}",
        );
        let body = func_body(&ast, "g");
        // The guard survives verbatim; only the expression becomes a component.
        assert!(body.children().any(|c| c.kind() == NodeKind::GuardStmt));
        let kids: Vec<_> = body.children().map(|c| c.kind()).collect();
        assert_eq!(
            kids,
            vec![NodeKind::GuardStmt, NodeKind::LetDecl, NodeKind::ReturnStmt]
        );
    }

    #[test]
    fn unsupported_construct_in_builder_body_is_diagnosed() {
        // A `do` block is not a builder component and cannot be lowered; with no
        // runtime fallback it must surface as a diagnostic, never run silently.
        let diags = diags_of(&format!(
            "{STRING_BUILDER}@B\nfunc g() -> String {{\n \"a\"\n do {{ }}\n}}"
        ));
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("not supported in a result-builder body")),
            "{diags:?}"
        );
    }

    #[test]
    fn unsupported_construct_in_contextual_closure_is_diagnosed() {
        // The contextual-closure path must validate too: a `while` inside a
        // closure passed to a @Builder parameter must be diagnosed, not run as a
        // plain closure.
        let diags = diags_of(&format!(
            "{STRING_BUILDER}func wrap(@B _ c: () -> String) -> String {{ c() }}\n\
             let r = wrap {{ \"a\"\n while false {{ \"x\" }}\n \"b\" }}"
        ));
        assert!(
            diags.iter().any(|d| d.message.contains("'while'/'repeat'")),
            "{diags:?}"
        );
    }

    #[test]
    fn bare_if_without_build_optional_is_diagnosed() {
        // COND_BUILDER-without-buildOptional: a bare `if` needs buildOptional.
        let src = "@resultBuilder\nstruct B {\n\
            static func buildExpression(_ v: String) -> String { v }\n\
            static func buildBlock(_ p: String...) -> String { \"\" }\n\
            static func buildEither(first: String) -> String { first }\n\
            static func buildEither(second: String) -> String { second }\n}\n\
            @B\nfunc g(_ b: Bool) -> String {\n if b { \"x\" }\n}";
        let diags = diags_of(src);
        assert!(
            diags.iter().any(|d| d.message.contains("buildOptional")),
            "bare if without buildOptional should be diagnosed: {diags:?}"
        );
    }

    #[test]
    fn if_else_without_build_either_is_diagnosed() {
        // A builder with buildOptional but no buildEither cannot lower if/else.
        let src = "@resultBuilder\nstruct B {\n\
            static func buildExpression(_ v: String) -> String { v }\n\
            static func buildBlock(_ p: String...) -> String { \"\" }\n\
            static func buildOptional(_ p: String?) -> String { p ?? \"\" }\n}\n\
            @B\nfunc g(_ b: Bool) -> String {\n if b { \"x\" } else { \"y\" }\n}";
        let diags = diags_of(src);
        assert!(
            diags.iter().any(|d| d.message.contains("buildEither")),
            "if/else without buildEither should be diagnosed: {diags:?}"
        );
    }

    #[test]
    fn enum_result_builder_is_diagnosed() {
        let diags = diags_of(
            "@resultBuilder\nenum B {\n static func buildBlock(_ p: String...) -> String { \"\" } }",
        );
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("must be a 'struct' or 'class'")),
            "{diags:?}"
        );
    }
}
