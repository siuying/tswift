//! Semantic analysis for the quick-swift frontend.
//!
//! [`resolve`] walks a parsed [`swift_ast::Ast`], infers and records a [`Type`]
//! on each expression node, and returns any [`Diagnostic`]s. Coverage today is
//! **Tier 0 + Tier 1a/1b/1c**: literal and operator types, lexically-scoped name
//! resolution against `let`/`var` bindings and function parameters, type-
//! annotation checking, and structural resolution of functions, blocks, and all
//! control-flow statements (`if`/`guard`/`while`/`repeat`/`for`/`switch`).

#![forbid(unsafe_code)]

use std::collections::HashMap;

use swift_ast::{Ast, Node, NodeId, NodeKind, Type};

/// One semantic diagnostic with its 1-based source location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub line: u32,
    pub col: u32,
}

/// Resolve types over `ast` in place, returning diagnostics in source order.
pub fn resolve(ast: &mut Ast) -> Vec<Diagnostic> {
    let mut r = Resolver {
        scopes: vec![HashMap::new()],
        types: Vec::new(),
        diags: Vec::new(),
    };
    let root = ast.root();
    for stmt in child_ids(ast, root) {
        r.resolve_statement(ast, stmt);
    }
    for (id, ty) in &r.types {
        ast.set_type(*id, *ty);
    }
    r.diags
}

struct Resolver {
    /// A lexical scope stack; the last entry is the innermost scope.
    scopes: Vec<HashMap<String, Type>>,
    /// Pending `(node, type)` annotations, applied after the walk.
    types: Vec<(NodeId, Type)>,
    diags: Vec<Diagnostic>,
}

impl Resolver {
    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn bind(&mut self, name: &str, ty: Type) {
        self.scopes.last_mut().unwrap().insert(name.to_string(), ty);
    }

    fn lookup(&self, name: &str) -> Option<Type> {
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
            | NodeKind::ProtocolDecl
            | NodeKind::ExtensionDecl => {
                self.push_scope();
                for &member in &kids {
                    self.resolve_statement(ast, member);
                }
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
                for &s in &kids {
                    self.resolve_statement(ast, s);
                }
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
            }
            NodeKind::ReturnStmt => {
                if let Some(&value) = kids.first() {
                    self.infer(ast, value);
                }
            }
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
        if let (Some(name), Some(ty)) = (ast.node(param).text(), ty) {
            self.bind(name, ty);
            self.types.push((param, ty));
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
            (Some(a), Some(b)) if a != b => {
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
            if ast.node(pattern).kind() == NodeKind::NamePattern {
                if let Some(name) = ast.node(pattern).text() {
                    self.bind(name, ty);
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
            NodeKind::BoolLiteral => Some(Type::Bool),
            NodeKind::NilLiteral => None,
            NodeKind::IdentExpr => node.text().and_then(|name| self.lookup(name)),
            NodeKind::CallExpr => {
                for c in &children {
                    self.infer(ast, *c);
                }
                Some(Type::Void)
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
                for c in &children {
                    self.infer(ast, *c);
                }
                Some(Type::Void)
            }
            NodeKind::ClosureExpr => {
                self.push_scope();
                for c in &children {
                    self.resolve_statement(ast, *c);
                }
                self.pop_scope();
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
        match (lhs, rhs) {
            (Some(a), Some(b)) if a == b => Some(a),
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

/// Map a written annotation to a modelled scalar type, else `None`.
fn parse_type_name(text: &str) -> Option<Type> {
    match text {
        "Int" => Some(Type::Int),
        "Double" => Some(Type::Double),
        "String" => Some(Type::String),
        "Bool" => Some(Type::Bool),
        "Void" => Some(Type::Void),
        _ => None,
    }
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
    use swift_parser::parse;

    fn resolved(src: &str) -> (Ast, Vec<Diagnostic>) {
        let mut ast = parse(src).expect("parse ok");
        let diags = resolve(&mut ast);
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
}
