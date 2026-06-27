//! Semantic analysis for the tswift frontend.
//!
//! [`analyze`] runs an ordered pipeline of [`passes`] over a parsed
//! [`tswift_ast::Ast`], sharing one [`Symbols`] declaration registry. The sole
//! pass today is [`annotate`], which walks the tree, infers and records a
//! [`Type`] on each expression node, and returns any [`Diagnostic`]s. Coverage
//! **Tier 0 + Tier 1a/1b/1c**: literal and operator types, lexically-scoped name
//! resolution against `let`/`var` bindings and function parameters, type-
//! annotation checking, and structural resolution of functions, blocks, and all
//! control-flow statements (`if`/`guard`/`while`/`repeat`/`for`/`switch`).

#![forbid(unsafe_code)]

use std::collections::HashMap;

use tswift_ast::{Ast, Node, NodeId, NodeKind, Type};

mod passes;
mod symbols;

pub use passes::analyze;
use symbols::Symbols;

/// One semantic diagnostic with its 1-based source location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub line: u32,
    pub col: u32,
}

/// The annotate pass: resolve names and record a [`Type`] on each expression
/// node of `ast` in place, returning diagnostics in source order. Reads
/// declarations from the shared `symbols` registry; does not rewrite the tree.
pub(crate) fn annotate(ast: &mut Ast, symbols: &Symbols) -> Vec<Diagnostic> {
    let mut r = Resolver {
        scopes: vec![HashMap::new()],
        types: Vec::new(),
        diags: Vec::new(),
        in_type_body: false,
        symbols,
    };
    let root = ast.root();
    r.prebind_func_decls(ast, root);
    for stmt in child_ids(ast, root) {
        r.resolve_statement(ast, stmt);
    }
    for (id, ty) in &r.types {
        ast.set_type(*id, *ty);
    }
    r.diags
}

/// One resolved name binding: its (optionally known) type and whether it may be
/// reassigned (`var`/`inout`/parameter) or is a `let` constant.
#[derive(Clone, Copy)]
struct Binding {
    ty: Option<Type>,
    mutable: bool,
}

struct Resolver<'a> {
    /// A lexical scope stack; the last entry is the innermost scope.
    scopes: Vec<HashMap<String, Binding>>,
    /// Pending `(node, type)` annotations, applied after the walk.
    types: Vec<(NodeId, Type)>,
    diags: Vec<Diagnostic>,
    /// True while binding the direct members of a nominal type. Stored `let`
    /// properties are bound as mutable here: assigning to them is legal inside
    /// the type's initializer, so the local-`let` constant check must not fire.
    in_type_body: bool,
    /// The program's declaration registry: enum cases, function return types,
    /// and other "what does the program declare?" facts, collected once and
    /// shared across passes.
    symbols: &'a Symbols,
}

impl Resolver<'_> {
    /// Bind the functions declared directly under `parent` into the current
    /// scope, reading each return type from the declaration registry. Binding
    /// (not the registry) controls visibility, so block-local functions stay
    /// scoped to their block.
    fn prebind_func_decls(&mut self, ast: &Ast, parent: NodeId) {
        for child in child_ids(ast, parent) {
            if ast.node(child).kind() != NodeKind::FuncDecl {
                continue;
            }
            if let Some(name) = ast.node(child).text() {
                let ret = self.symbols.func_return(name).unwrap_or(Type::Void);
                self.bind(name, Some(ret), false);
            }
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn bind(&mut self, name: &str, ty: Option<Type>, mutable: bool) {
        let mutable = mutable || self.in_type_body;
        self.scopes
            .last_mut()
            .unwrap()
            .insert(name.to_string(), Binding { ty, mutable });
    }

    fn lookup(&self, name: &str) -> Option<Type> {
        self.lookup_binding(name).and_then(|b| b.ty)
    }

    fn lookup_binding(&self, name: &str) -> Option<Binding> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn resolve_statement(&mut self, ast: &Ast, stmt: NodeId) {
        let kids = child_ids(ast, stmt);
        match ast.node(stmt).kind() {
            NodeKind::LetDecl | NodeKind::VarDecl => self.resolve_binding(ast, stmt),
            NodeKind::FuncDecl => self.resolve_func(ast, stmt, &kids),
            NodeKind::StructDecl
            | NodeKind::EnumDecl
            | NodeKind::ClassDecl
            | NodeKind::ProtocolDecl => {
                self.push_scope();
                let saved = self.in_type_body;
                self.in_type_body = true;
                for &member in &kids {
                    self.resolve_statement(ast, member);
                }
                self.in_type_body = saved;
                self.pop_scope();
            }
            NodeKind::ExtensionDecl => {
                self.push_scope();
                let saved = self.in_type_body;
                self.in_type_body = true;
                for &member in &kids {
                    if is_stored_property(ast, member) {
                        let n = ast.node(member);
                        self.diags.push(Diagnostic {
                            message: "extensions must not contain stored properties".to_string(),
                            line: n.line(),
                            col: n.col(),
                        });
                    }
                    self.resolve_statement(ast, member);
                }
                self.in_type_body = saved;
                self.pop_scope();
            }
            // Type-level declarations carry no value bindings to resolve.
            NodeKind::AssociatedTypeDecl | NodeKind::TypeAliasDecl | NodeKind::GenericParam => {}
            NodeKind::DeinitDecl => {
                for &c in &kids {
                    self.resolve_statement(ast, c);
                }
            }
            NodeKind::InitDecl | NodeKind::SubscriptDecl => {
                self.push_scope();
                for &c in &kids {
                    match ast.node(c).kind() {
                        NodeKind::Param => self.bind_param(ast, c),
                        NodeKind::TypeRef => {}
                        _ => self.resolve_statement(ast, c),
                    }
                }
                self.pop_scope();
            }
            NodeKind::Accessor => {
                for &c in &kids {
                    self.resolve_statement(ast, c);
                }
            }
            NodeKind::EnumCaseDecl => {
                for &c in &kids {
                    self.infer(ast, c);
                }
            }
            NodeKind::Block => {
                self.push_scope();
                // A block opens a local scope where `let` constants are checked.
                let saved = self.in_type_body;
                self.in_type_body = false;
                self.prebind_func_decls(ast, stmt);
                for &s in &kids {
                    self.resolve_statement(ast, s);
                }
                self.in_type_body = saved;
                self.pop_scope();
            }
            // `if`/`guard`/`while`: conditions (expressions or `let` bindings),
            // then their block(s) / nested `if`.
            NodeKind::IfStmt | NodeKind::GuardStmt | NodeKind::WhileStmt => {
                for &c in &kids {
                    self.resolve_statement(ast, c);
                }
            }
            NodeKind::RepeatStmt => {
                if let Some(&body) = kids.first() {
                    self.resolve_statement(ast, body);
                }
                if let Some(&cond) = kids.get(1) {
                    self.infer(ast, cond);
                }
            }
            NodeKind::ForStmt => {
                // pattern, iterable, [where], body block
                self.push_scope();
                if let Some(&iterable) = kids.get(1) {
                    self.infer(ast, iterable);
                }
                for &c in &kids[2..] {
                    if ast.node(c).kind() == NodeKind::Block {
                        self.resolve_statement(ast, c);
                    } else {
                        self.infer(ast, c); // where-expr
                    }
                }
                self.pop_scope();
            }
            NodeKind::SwitchStmt => {
                if let Some(&subject) = kids.first() {
                    self.infer(ast, subject);
                }
                for &clause in &kids[1..] {
                    self.resolve_case_clause(ast, clause);
                }
                self.check_switch_exhaustiveness(ast, stmt, &kids);
            }
            NodeKind::ReturnStmt | NodeKind::ThrowStmt => {
                if let Some(&value) = kids.first() {
                    self.infer(ast, value);
                }
            }
            NodeKind::DoStmt => {
                for &c in &kids {
                    self.resolve_statement(ast, c);
                }
            }
            NodeKind::CatchClause => {
                // The catch pattern bindings are visible in the catch body.
                self.push_scope();
                for &c in &kids {
                    self.resolve_statement(ast, c);
                }
                self.pop_scope();
            }
            NodeKind::DeferStmt => {
                if let Some(&body) = kids.first() {
                    self.resolve_statement(ast, body);
                }
            }
            NodeKind::CompilerDirective => {
                for &c in &kids {
                    self.resolve_statement(ast, c);
                }
            }
            NodeKind::OperatorDecl | NodeKind::PrecedenceGroupDecl => {}
            NodeKind::BreakStmt | NodeKind::ContinueStmt | NodeKind::FallthroughStmt => {}
            // Expression statements and assignments.
            _ => {
                self.infer(ast, stmt);
            }
        }
    }

    fn resolve_case_clause(&mut self, ast: &Ast, clause: NodeId) {
        self.push_scope();
        for c in child_ids(ast, clause) {
            if ast.node(c).kind() == NodeKind::Block {
                self.resolve_statement(ast, c);
            } else if ast.node(c).kind() != NodeKind::NamePattern {
                self.infer(ast, c); // value-pattern items and the where-expr
            }
        }
        self.pop_scope();
    }

    /// Diagnose a `switch` over an enum that omits cases and has no `default`.
    ///
    /// The check is deliberately *sound* (never a false positive): it fires only
    /// when the subject enum is unambiguously identified from the case patterns,
    /// no `default`/`@unknown default` clause is present, and the set of
    /// irrefutably-covered cases is a strict subset of the enum's cases.
    fn check_switch_exhaustiveness(&mut self, ast: &Ast, switch: NodeId, kids: &[NodeId]) {
        let clauses = &kids[1..];
        // A `default` (or `@unknown default`) clause makes the switch exhaustive.
        if clauses
            .iter()
            .any(|&c| ast.node(c).text() == Some("default"))
        {
            return;
        }

        // Gather the enum-case names referenced by every clause's patterns, plus
        // those covered irrefutably (no `where` guard, irrefutable payloads). A
        // catch-all pattern (`_` / bare name binding) covers everything.
        let mut referenced: Vec<String> = Vec::new();
        let mut covered: Vec<String> = Vec::new();
        for &clause in clauses {
            let items = child_ids(ast, clause);
            let guarded = items
                .iter()
                .any(|&c| ast.node(c).kind() == NodeKind::WhereClause);
            for &item in &items {
                match ast.node(item).kind() {
                    NodeKind::EnumCasePattern => {
                        if let Some(name) = ast.node(item).text() {
                            referenced.push(name.to_string());
                            if !guarded && self.payload_is_irrefutable(ast, item) {
                                covered.push(name.to_string());
                            }
                        }
                    }
                    // A bare `case _:` or `case let x:` (unguarded) is a
                    // catch-all: the switch is exhaustive regardless of cases.
                    NodeKind::WildcardPattern | NodeKind::NamePattern if !guarded => return,
                    _ => {}
                }
            }
        }

        // Identify the subject enum: the one enum whose case set contains every
        // referenced case name. A type-qualified pattern (`Direction.north`)
        // does not narrow this — the parser keeps only the case name — so the
        // ambiguity guard below is what keeps the check sound. Ambiguity or a
        // non-enum switch → no diagnostic.
        if referenced.is_empty() {
            return;
        }
        let fits: Vec<(&String, &Vec<String>)> = self
            .symbols
            .enums()
            .filter(|(_, cases)| referenced.iter().all(|r| cases.iter().any(|c| c == r)))
            .collect();
        let [(enum_name, all_cases)] = fits.as_slice() else {
            return; // no match, or ambiguous (multiple enums fit) — stay silent.
        };

        let missing: Vec<String> = all_cases
            .iter()
            .filter(|c| !covered.iter().any(|cov| cov == *c))
            .cloned()
            .collect();
        if missing.is_empty() {
            return;
        }
        let enum_name = (*enum_name).clone();
        let list = missing
            .iter()
            .map(|c| format!("'.{c}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let n = ast.node(switch);
        self.diags.push(Diagnostic {
            message: format!(
                "switch must be exhaustive: add missing case(s) {list} for enum '{enum_name}' or a 'default' clause"
            ),
            line: n.line(),
            col: n.col(),
        });
    }

    /// Whether an enum-case pattern's payload sub-patterns are all irrefutable
    /// (wildcards or plain name bindings), so the pattern matches the case for
    /// every payload value. Literal/range/nested-enum payloads are refutable.
    fn payload_is_irrefutable(&self, ast: &Ast, pattern: NodeId) -> bool {
        child_ids(ast, pattern)
            .iter()
            .all(|&sub| match ast.node(sub).kind() {
                NodeKind::WildcardPattern | NodeKind::NamePattern => true,
                NodeKind::TuplePattern => self.payload_is_irrefutable(ast, sub),
                _ => false,
            })
    }

    fn resolve_func(&mut self, ast: &Ast, decl: NodeId, kids: &[NodeId]) {
        self.push_scope();
        let mut ret_ty = None;
        let mut body = None;
        for &c in kids {
            match ast.node(c).kind() {
                NodeKind::Param => self.bind_param(ast, c),
                NodeKind::TypeRef => ret_ty = ast.node(c).text().and_then(parse_type_name),
                NodeKind::Block => body = Some(c),
                _ => {}
            }
        }
        if let Some(t) = ret_ty {
            self.types.push((decl, t));
        }
        if let Some(b) = body {
            self.resolve_statement(ast, b);
        }
        self.pop_scope();
    }

    fn bind_param(&mut self, ast: &Ast, param: NodeId) {
        let kids = child_ids(ast, param);
        let ty = kids
            .iter()
            .find(|c| ast.node(**c).kind() == NodeKind::TypeRef)
            .and_then(|c| ast.node(*c).text())
            .and_then(parse_type_name);
        if let Some(name) = ast.node(param).text() {
            self.bind(name, ty, true);
            if let Some(ty) = ty {
                self.types.push((param, ty));
            }
        }
        // Resolve a default-value expression, if any.
        for &c in &kids {
            if ast.node(c).kind() != NodeKind::TypeRef {
                self.infer(ast, c);
            }
        }
    }

    /// A `let`/`var` declaration: children are `pattern [, type_ref] [, init]`.
    fn resolve_binding(&mut self, ast: &Ast, decl: NodeId) {
        let kids = child_ids(ast, decl);
        let pattern = kids[0];

        let mut annotation = None;
        let mut init = None;
        for &child in &kids[1..] {
            if ast.node(child).kind() == NodeKind::TypeRef {
                annotation = ast.node(child).text().and_then(parse_type_name);
            } else {
                init = Some(child);
            }
        }

        let init_ty = init.and_then(|e| self.infer(ast, e));

        let bound_ty = match (annotation, init_ty) {
            // `Void` from an initializer means "type not modelled" (e.g. a method
            // call the skeleton sema cannot resolve), not a real mismatch.
            // An integer literal in a floating context coerces to the annotation
            // (`let r: Double = 5`), so it is not a mismatch.
            (Some(a), Some(b)) if a != b && b != Type::Void && !is_coercible(b, a) => {
                let n = ast.node(decl);
                self.diags.push(Diagnostic {
                    message: format!(
                        "cannot convert value of type '{}' to specified type '{}'",
                        b.name(),
                        a.name()
                    ),
                    line: n.line(),
                    col: n.col(),
                });
                Some(a)
            }
            (Some(a), _) => Some(a),
            (None, b) => b,
        };

        if let Some(ty) = bound_ty {
            self.types.push((decl, ty));
        }
        if ast.node(pattern).kind() == NodeKind::NamePattern {
            if let Some(name) = ast.node(pattern).text() {
                let mutable = ast.node(decl).kind() == NodeKind::VarDecl;
                self.bind(name, bound_ty, mutable);
                if let Some(ty) = bound_ty {
                    self.types.push((pattern, ty));
                }
            }
        }
    }

    /// Infer and record the type of an expression subtree (post-order).
    fn infer(&mut self, ast: &Ast, id: NodeId) -> Option<Type> {
        let node = ast.node(id);
        let kind = node.kind();
        let children = child_ids(ast, id);

        let ty = match kind {
            NodeKind::IntegerLiteral => Some(Type::Int),
            NodeKind::FloatLiteral => Some(Type::Double),
            NodeKind::StringLiteral => Some(Type::String),
            NodeKind::RegexLiteral => Some(Type::Regex),
            NodeKind::BoolLiteral => Some(Type::Bool),
            NodeKind::NilLiteral => None,
            NodeKind::IdentExpr => node.text().and_then(|name| self.lookup(name)),
            NodeKind::CallExpr => {
                let callee_ty = children.first().and_then(|c| self.infer(ast, *c));
                for c in children.iter().skip(1) {
                    self.infer(ast, *c);
                }
                callee_ty.or(Some(Type::Void))
            }
            NodeKind::PrefixExpr => {
                let operand = children.first().and_then(|c| self.infer(ast, *c));
                match node.text() {
                    Some("!") => Some(Type::Bool),
                    _ => operand,
                }
            }
            NodeKind::BinaryExpr => self.infer_binary(ast, &node, &children),
            NodeKind::TernaryExpr => {
                if let Some(c) = children.first() {
                    self.infer(ast, *c);
                }
                let then_ty = children.get(1).and_then(|c| self.infer(ast, *c));
                let else_ty = children.get(2).and_then(|c| self.infer(ast, *c));
                match (then_ty, else_ty) {
                    (Some(a), Some(b)) if a == b => Some(a),
                    _ => then_ty.or(else_ty),
                }
            }
            // `if` used as an expression: type is the matching branch type.
            NodeKind::IfStmt => {
                self.resolve_statement(ast, id);
                None
            }
            NodeKind::AssignExpr => {
                if let Some(&lhs) = children.first() {
                    self.check_assignable(ast, lhs);
                }
                for c in &children {
                    self.infer(ast, *c);
                }
                Some(Type::Void)
            }
            NodeKind::ClosureExpr => {
                self.push_scope();
                let saved = self.in_type_body;
                self.in_type_body = false;
                for c in &children {
                    self.resolve_statement(ast, *c);
                }
                self.in_type_body = saved;
                self.pop_scope();
                None
            }
            NodeKind::TryExpr => children.first().and_then(|c| self.infer(ast, *c)),
            NodeKind::CompilerDirective => {
                for c in &children {
                    self.infer(ast, *c);
                }
                None
            }
            NodeKind::CastExpr => {
                if let Some(c) = children.first() {
                    self.infer(ast, *c);
                }
                match node.text() {
                    Some("is") => Some(Type::Bool),
                    Some("as?") => None,
                    _ => children
                        .get(1)
                        .and_then(|c| ast.node(*c).text())
                        .and_then(parse_type_name),
                }
            }
            _ => {
                for c in &children {
                    self.infer(ast, *c);
                }
                None
            }
        };

        if let Some(t) = ty {
            self.types.push((id, t));
        }
        ty
    }

    /// Diagnose assignment to a `let` constant when the assignment target is a
    /// bare name bound by `let`.
    fn check_assignable(&mut self, ast: &Ast, lhs: NodeId) {
        let node = ast.node(lhs);
        if node.kind() != NodeKind::IdentExpr {
            return;
        }
        let Some(name) = node.text() else { return };
        if let Some(binding) = self.lookup_binding(name) {
            if !binding.mutable {
                self.diags.push(Diagnostic {
                    message: format!("cannot assign to value: '{name}' is a 'let' constant"),
                    line: node.line(),
                    col: node.col(),
                });
            }
        }
    }

    fn infer_binary(&mut self, ast: &Ast, node: &Node<'_>, children: &[NodeId]) -> Option<Type> {
        let lhs = children.first().and_then(|c| self.infer(ast, *c));
        let rhs = children.get(1).and_then(|c| self.infer(ast, *c));
        let op = node.text().unwrap_or("?");

        if is_comparison(op) || is_logical(op) {
            return Some(Type::Bool);
        }
        if is_range(op) {
            return None;
        }
        if op == "??" {
            return rhs.or(lhs);
        }
        // `Void` here means "could not be inferred" (e.g. a method call whose
        // return type the skeleton sema does not model), not a real operand
        // type. Treat it as unknown rather than reporting a false mismatch.
        let lhs = lhs.filter(|t| *t != Type::Void);
        let rhs = rhs.filter(|t| *t != Type::Void);
        match (lhs, rhs) {
            (Some(a), Some(b)) if a == b => Some(a),
            // Mixed integer/floating arithmetic: an integer literal coerces to
            // the floating operand, so the result is `Double` (matches the
            // runtime's numeric promotion).
            (Some(a), Some(b)) if is_coercible(a, b) => Some(b),
            (Some(a), Some(b)) if is_coercible(b, a) => Some(a),
            (Some(a), Some(b)) => {
                self.diags.push(Diagnostic {
                    message: format!(
                        "binary operator '{op}' cannot combine {} and {}",
                        a.name(),
                        b.name()
                    ),
                    line: node.line(),
                    col: node.col(),
                });
                Some(a)
            }
            _ => lhs.or(rhs),
        }
    }
}

fn child_ids(ast: &Ast, id: NodeId) -> Vec<NodeId> {
    ast.node(id).children().map(|c| c.id()).collect()
}

/// Whether `member` is an instance stored property — a `let`/`var` with neither
/// a `static`/`class` modifier nor an accessor block (which would make it
/// computed). Such properties are illegal inside an extension.
fn is_stored_property(ast: &Ast, member: NodeId) -> bool {
    let node = ast.node(member);
    if !matches!(node.kind(), NodeKind::VarDecl | NodeKind::LetDecl) {
        return false;
    }
    if node
        .modifiers()
        .iter()
        .any(|m| m == "static" || m == "class")
    {
        return false;
    }
    !node.children().any(|c| c.kind() == NodeKind::Accessor)
}

/// Map a written annotation to a modelled scalar type, else `None`.
fn parse_type_name(text: &str) -> Option<Type> {
    match text {
        "Int" => Some(Type::Int),
        "Double" => Some(Type::Double),
        "String" => Some(Type::String),
        "Bool" => Some(Type::Bool),
        "Void" => Some(Type::Void),
        "Regex" => Some(Type::Regex),
        _ => None,
    }
}

/// Whether a value of type `from` implicitly coerces to `to` in an annotated
/// context — currently an integer literal widening to a floating type, matching
/// Swift's `ExpressibleByIntegerLiteral` conversion for `Double`/`Float`.
fn is_coercible(from: Type, to: Type) -> bool {
    matches!((from, to), (Type::Int, Type::Double))
}

fn is_comparison(op: &str) -> bool {
    matches!(op, "==" | "!=" | "<" | ">" | "<=" | ">=" | "===" | "!==")
}

fn is_logical(op: &str) -> bool {
    matches!(op, "&&" | "||")
}

fn is_range(op: &str) -> bool {
    matches!(op, "..<" | "...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_parser::parse;

    fn resolved(src: &str) -> (Ast, Vec<Diagnostic>) {
        let mut ast = parse(src).expect("parse ok");
        let diags = analyze(&mut ast);
        (ast, diags)
    }

    fn first_binding_init_type(ast: &Ast) -> Option<&'static str> {
        let decl = ast.node(ast.root()).children().next().unwrap();
        decl.children().last().unwrap().type_name()
    }

    #[test]
    fn types_integer_arithmetic_as_int() {
        let (ast, diags) = resolved("let x = 1 + 2");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(first_binding_init_type(&ast), Some("Int"));
    }

    #[test]
    fn literal_types_cover_each_scalar() {
        assert_eq!(
            first_binding_init_type(&resolved("let a = 3.14").0),
            Some("Double")
        );
        assert_eq!(
            first_binding_init_type(&resolved("let a = true").0),
            Some("Bool")
        );
        assert_eq!(
            first_binding_init_type(&resolved(r#"let a = "hi""#).0),
            Some("String")
        );
    }

    #[test]
    fn comparison_and_logical_are_bool() {
        assert_eq!(
            first_binding_init_type(&resolved("let a = 1 < 2").0),
            Some("Bool")
        );
        assert_eq!(
            first_binding_init_type(&resolved("let a = true && false").0),
            Some("Bool")
        );
    }

    #[test]
    fn identifiers_resolve_against_bindings() {
        let (ast, diags) = resolved("let x = 10\nlet y = x + 5");
        assert!(diags.is_empty(), "{diags:?}");
        let second = ast.node(ast.root()).children().nth(1).unwrap();
        assert_eq!(second.children().last().unwrap().type_name(), Some("Int"));
    }

    #[test]
    fn annotation_is_recorded_and_checked() {
        let (ast, diags) = resolved("let x: Double = 2.5");
        assert!(diags.is_empty(), "{diags:?}");
        let decl = ast.node(ast.root()).children().next().unwrap();
        assert_eq!(decl.type_name(), Some("Double"));
    }

    #[test]
    fn annotation_mismatch_diagnoses() {
        let (_ast, diags) = resolved(r#"let x: Int = "oops""#);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("cannot convert"), "{diags:?}");
    }

    #[test]
    fn integer_literal_coerces_to_double_without_diagnostic() {
        // `let r: Double = 5` is an integer literal in a floating context.
        let (ast, diags) = resolved("let r: Double = 5");
        assert!(diags.is_empty(), "{diags:?}");
        let decl = ast.node(ast.root()).children().next().unwrap();
        assert_eq!(decl.type_name(), Some("Double"));
    }

    #[test]
    fn mixed_int_double_arithmetic_is_double() {
        // `d / 4` with `d: Double` promotes the integer operand.
        let (_ast, diags) = resolved("let d: Double = 10\nlet half = d / 4");
        assert!(diags.is_empty(), "{diags:?}");
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
    }

    #[test]
    fn mixed_operand_types_diagnose() {
        let (_ast, diags) = resolved(r#"let a = 1 + "x""#);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("cannot combine"), "{diags:?}");
    }

    #[test]
    fn prefix_not_is_bool() {
        let (ast, _diags) = resolved("let a = !true");
        assert_eq!(first_binding_init_type(&ast), Some("Bool"));
    }

    // --- Tier 1b/1c ---

    #[test]
    fn recursive_function_calls_use_declared_return_type() {
        let (_ast, diags) = resolved(
            "func factorial(_ n: Int) -> Int { return n == 0 ? 1 : n * factorial(n - 1) }",
        );
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn function_params_are_in_scope_in_the_body() {
        let (ast, diags) = resolved("func add(a: Int, b: Int) -> Int { return a + b }");
        assert!(diags.is_empty(), "{diags:?}");
        // The return value `a + b` types as Int via the params.
        let func = ast.node(ast.root()).children().next().unwrap();
        assert_eq!(func.type_name(), Some("Int")); // recorded return type
        let body = func.children().last().unwrap();
        let ret = body.children().next().unwrap();
        assert_eq!(ret.children().next().unwrap().type_name(), Some("Int"));
    }

    #[test]
    fn locals_do_not_leak_out_of_function_scope() {
        // `inner` is local to f; referencing it afterwards stays unresolved (None),
        // which simply yields no type rather than resolving to the local.
        let (ast, diags) = resolved("func f() { let inner = 1 }\nlet x = 2");
        assert!(diags.is_empty(), "{diags:?}");
        let x = ast.node(ast.root()).children().nth(1).unwrap();
        assert_eq!(x.type_name(), Some("Int"));
    }

    #[test]
    fn control_flow_resolves_without_diagnostics() {
        let src = "let xs = 0\n\
                   if xs > 0 { let a = xs + 1 } else { let b = xs - 1 }\n\
                   while xs < 10 { let c = xs }\n\
                   for i in 0 ..< 3 where i > 0 { let d = i }\n\
                   switch xs { case 0: break\ndefault: break }";
        let (_ast, diags) = resolved(src);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn binding_inside_block_is_scoped() {
        // `a` is bound inside the if-block and used there; the outer `let r`
        // does not see it, so no spurious diagnostics arise.
        let (_ast, diags) = resolved("if true { let a = 1\nlet b = a + 1 }");
        assert!(diags.is_empty(), "{diags:?}");
    }

    // --- Tier 2 ---

    #[test]
    fn value_types_resolve_without_diagnostics() {
        let src = "struct Point {\n\
                   var x: Int\n\
                   var y: Int\n\
                   func sumSquares() -> Int { return x * x + y * y }\n\
                   var magnitudeHint: Int { return x + y }\n\
                   }\n\
                   enum Suit: Int {\n\
                   case hearts = 1, spades\n\
                   }";
        let (_ast, diags) = resolved(src);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn method_params_and_body_resolve() {
        let src = "struct Calc { func add(a: Int, b: Int) -> Int { return a + b } }";
        let (_ast, diags) = resolved(src);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn if_let_binding_resolves() {
        let (_ast, diags) = resolved("var maybe = 1\nif let value = maybe { let doubled = value }");
        assert!(diags.is_empty(), "{diags:?}");
    }

    // --- Tier 3 ---

    #[test]
    fn classes_and_closures_resolve() {
        let src = "class Animal {\n\
                   let name: String\n\
                   init(name: String) { self.name = name }\n\
                   deinit { }\n\
                   }\n\
                   let twice = numbers.map { x in x + x }";
        let (_ast, diags) = resolved(src);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn is_cast_is_bool() {
        let (ast, _diags) = resolved("let flag = value is Int");
        assert_eq!(first_binding_init_type(&ast), Some("Bool"));
    }

    // --- Tier 4 ---

    #[test]
    fn errors_and_directives_resolve() {
        let src = "func load() {\n\
                   defer { close() }\n\
                   do {\n\
                   let data = try read()\n\
                   process(data)\n\
                   } catch let error {\n\
                   report(error)\n\
                   }\n\
                   }";
        let (_ast, diags) = resolved(src);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn conditional_compilation_binding_is_visible() {
        // The active branch's binding must register in the enclosing scope.
        let src = "#if DEBUG\n let level = 1\n #else\n let level = 0\n #endif\n\
                   let next = level + 1";
        let (_ast, diags) = resolved(src);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn assigning_to_a_let_constant_is_diagnosed() {
        let (_ast, diags) = resolved("let limit = 10\nlimit = 20");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("'let' constant"), "{diags:?}");
        assert_eq!(diags[0].line, 2);
    }

    #[test]
    fn assigning_to_a_var_is_allowed() {
        let (_ast, diags) = resolved("var total = 0\ntotal = 5");
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn initializer_may_assign_to_a_let_stored_property() {
        // A `let` property is assignable inside the type's initializer; the
        // local-constant check must not fire on it.
        let src = "struct R {\n\
                   let id: Int\n\
                   init(_ i: Int) { id = i }\n\
                   }";
        let (_ast, diags) = resolved(src);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn extension_stored_property_is_diagnosed() {
        let src = "struct A { var x: Int }\n\
                   extension A { var pending: Int = 0 }";
        let (_ast, diags) = resolved(src);
        assert!(
            diags.iter().any(|d| d
                .message
                .contains("extensions must not contain stored properties")),
            "{diags:?}"
        );
    }

    #[test]
    fn extension_computed_property_is_allowed() {
        let src = "struct A { var x: Int }\n\
                   extension A { var doubled: Int { x } }";
        let (_ast, diags) = resolved(src);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn protocols_generics_extensions_resolve() {
        let src = "protocol Shape {\n\
                   var area: Double { get }\n\
                   func scaled(by factor: Double) -> Double\n\
                   }\n\
                   struct Square: Shape {\n\
                   let side: Double\n\
                   var area: Double { return side * side }\n\
                   func scaled(by factor: Double) -> Double { return side * factor }\n\
                   }\n\
                   extension Square {\n\
                   func describe() -> Double { return area }\n\
                   }\n\
                   func maxOf<T>(a: T, b: T, pick: T) -> T { return pick }";
        let (_ast, diags) = resolved(src);
        assert!(diags.is_empty(), "{diags:?}");
    }

    // --- Switch exhaustiveness ---

    fn exhaustiveness_diag(src: &str) -> Option<String> {
        let (_ast, diags) = resolved(src);
        diags
            .into_iter()
            .find(|d| d.message.contains("switch must be exhaustive"))
            .map(|d| d.message)
    }

    #[test]
    fn non_exhaustive_enum_switch_is_diagnosed() {
        let src = "enum D { case north, south, east, west }\n\
                   let d = D.north\n\
                   switch d { case .north: break }";
        let msg = exhaustiveness_diag(src).expect("expected a diagnostic");
        assert!(msg.contains("'.south'"), "{msg}");
        assert!(msg.contains("'.east'"), "{msg}");
        assert!(msg.contains("'.west'"), "{msg}");
    }

    #[test]
    fn exhaustive_enum_switch_has_no_diagnostic() {
        let src = "enum D { case north, south }\n\
                   let d = D.north\n\
                   switch d { case .north: break\ncase .south: break }";
        assert_eq!(exhaustiveness_diag(src), None);
    }

    #[test]
    fn comma_separated_cases_count_as_covered() {
        let src = "enum D { case a, b, c }\n\
                   switch D.a { case .a, .b, .c: break }";
        assert_eq!(exhaustiveness_diag(src), None);
    }

    #[test]
    fn default_clause_makes_switch_exhaustive() {
        let src = "enum D { case a, b, c }\n\
                   switch D.a { case .a: break\ndefault: break }";
        assert_eq!(exhaustiveness_diag(src), None);
    }

    #[test]
    fn unknown_default_makes_switch_exhaustive() {
        let src = "enum D { case a, b, c }\n\
                   switch D.a { case .a: break\n@unknown default: break }";
        assert_eq!(exhaustiveness_diag(src), None);
    }

    #[test]
    fn catch_all_binding_makes_switch_exhaustive() {
        let src = "enum D { case a, b, c }\n\
                   switch D.a { case .a: break\ncase let other: _ = other }";
        assert_eq!(exhaustiveness_diag(src), None);
    }

    #[test]
    fn where_guarded_case_does_not_cover() {
        // A `where`-guarded `.a` may fail, so `.a` is still missing.
        let src = "enum D { case a, b }\n\
                   let n = 1\n\
                   switch D.a { case .a where n > 0: break\ncase .b: break }";
        let msg = exhaustiveness_diag(src).expect("expected a diagnostic");
        assert!(msg.contains("'.a'"), "{msg}");
    }

    #[test]
    fn irrefutable_payload_binding_covers_case() {
        let src = "enum D { case a(Int), b }\n\
                   switch D.b { case .a(let x): _ = x\ncase .b: break }";
        assert_eq!(exhaustiveness_diag(src), None);
    }

    #[test]
    fn refutable_payload_does_not_cover_case() {
        // `.a(0)` only matches a specific payload, so `.a` is not fully covered.
        let src = "enum D { case a(Int), b }\n\
                   switch D.b { case .a(0): break\ncase .b: break }";
        let msg = exhaustiveness_diag(src).expect("expected a diagnostic");
        assert!(msg.contains("'.a'"), "{msg}");
    }

    #[test]
    fn integer_switch_is_not_treated_as_enum() {
        // No enum patterns → nothing to check; partial Int switch is fine here.
        let src = "let n = 3\nswitch n { case 0: break\ncase 1: break }";
        assert_eq!(exhaustiveness_diag(src), None);
    }
}
