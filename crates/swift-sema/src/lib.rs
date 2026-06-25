//! Semantic analysis for the quick-swift frontend.
//!
//! [`resolve`] walks a parsed [`swift_ast::Ast`], infers and records a [`Type`]
//! on each expression node, and returns any [`Diagnostic`]s. Coverage today is
//! **Tier 0 + Tier 1a**: literal types, the result types of unary/binary/ternary
//! operators, name resolution against `let`/`var` bindings (with type-annotation
//! checking), and the `Void` result of a `print(...)` call. Conformance, member
//! resolution, and richer types arrive in later tiers.

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
        env: HashMap::new(),
        types: Vec::new(),
        diags: Vec::new(),
    };
    let root = ast.root();
    // Top-level statements are processed in order so each binding is in scope
    // for the statements that follow it.
    for stmt in child_ids(ast, root) {
        r.resolve_statement(ast, stmt);
    }
    for (id, ty) in &r.types {
        ast.set_type(*id, *ty);
    }
    r.diags
}

struct Resolver {
    /// Name → type for the bindings currently in scope (top-level only for now).
    env: HashMap<String, Type>,
    /// Pending `(node, type)` annotations, applied after the walk.
    types: Vec<(NodeId, Type)>,
    diags: Vec<Diagnostic>,
}

impl Resolver {
    fn resolve_statement(&mut self, ast: &Ast, stmt: NodeId) {
        match ast.node(stmt).kind() {
            NodeKind::LetDecl | NodeKind::VarDecl => self.resolve_binding(ast, stmt),
            _ => {
                self.infer(ast, stmt);
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

        // Annotation wins; flag a contradiction when both are known and differ.
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
            self.bind_pattern(ast, pattern, ty);
        }
    }

    /// Record `name -> ty` for a simple name pattern. Tuple/wildcard patterns
    /// bind no scalar names in this tier.
    fn bind_pattern(&mut self, ast: &Ast, pattern: NodeId, ty: Type) {
        let node = ast.node(pattern);
        if node.kind() == NodeKind::NamePattern {
            if let Some(name) = node.text() {
                self.env.insert(name.to_string(), ty);
                self.types.push((pattern, ty));
            }
        }
    }

    /// Infer and record the type of an expression/statement subtree (post-order).
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
            NodeKind::IdentExpr => node.text().and_then(|name| self.env.get(name).copied()),
            NodeKind::CallExpr => {
                for c in &children {
                    self.infer(ast, *c);
                }
                Some(Type::Void) // `print(...)` and friends; refined in later tiers
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
                // cond ? then : else  — result is the (matching) branch type.
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
            NodeKind::AssignExpr => {
                for c in &children {
                    self.infer(ast, *c);
                }
                Some(Type::Void)
            }
            // Tuples, members, ranges, patterns, type refs, statements: no scalar
            // type in this tier — still recurse so nested expressions get typed.
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
            return None; // Range types are not modelled yet
        }
        if op == "??" {
            return rhs.or(lhs);
        }
        // Arithmetic / bitwise: operands must share a type.
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

/// Map a written annotation to a modelled scalar type, else `None` (arrays,
/// optionals, tuples, and unknown names are not modelled in this tier).
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

    /// Find the initializer type recorded on the first `let`/`var` binding.
    fn first_binding_init_type(ast: &Ast) -> Option<&'static str> {
        let decl = ast.node(ast.root()).children().next().unwrap();
        // last child is the initializer when present
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
        // second statement's initializer is `x + 5`, typed Int via `x`.
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
}
