//! Declaration registry — "what does the program declare?"
//!
//! [`Symbols`] is built in a single walk of the [`Ast`] and answers the
//! declaration queries that several analysis passes need without each pass
//! re-walking the tree:
//!
//! * [`Symbols::enum_cases`] / [`Symbols::enums`] — enum case lists, used by the
//!   `switch` exhaustiveness check.
//! * [`Symbols::func_return`] — a function's declared return type, used when
//!   pre-binding function names for call resolution.
//!
//! Collection lives here so that adding a new declaration query (for example the
//! result-builder transform's builder-method set) is a new field and getter on
//! one registry, not a new ad-hoc collector walk.

use std::collections::HashMap;

use tswift_ast::{Ast, NodeId, NodeKind, Type};

use crate::{child_ids, parse_type_name};

/// Every top-level fact about what a program declares, collected once.
#[derive(Debug, Default)]
pub(crate) struct Symbols {
    /// Each `enum` (including nested ones) by name, mapped to its case names in
    /// source order.
    enums: HashMap<String, Vec<String>>,
    /// Each function by name, mapped to its declared return type (`Void` when
    /// none is written). Keyed by name, not by node, so two same-named
    /// functions in different scopes collapse to the last one walked — the
    /// current tier resolves calls by return type only and does not model
    /// per-scope function shadowing.
    func_returns: HashMap<String, Type>,
}

impl Symbols {
    /// Collect every declaration in `ast` in a single recursive walk.
    pub(crate) fn collect(ast: &Ast) -> Self {
        let mut symbols = Symbols::default();
        symbols.walk(ast, ast.root());
        symbols
    }

    fn walk(&mut self, ast: &Ast, parent: NodeId) {
        for child in child_ids(ast, parent) {
            match ast.node(child).kind() {
                NodeKind::EnumDecl => {
                    if let Some(name) = ast.node(child).text() {
                        let cases = collect_enum_cases(ast, child);
                        if !cases.is_empty() {
                            self.enums.insert(name.to_string(), cases);
                        }
                    }
                }
                NodeKind::FuncDecl => {
                    if let Some(name) = ast.node(child).text() {
                        self.func_returns
                            .insert(name.to_string(), func_return_type(ast, child));
                    }
                }
                _ => {}
            }
            // Recurse: declarations may be nested inside other types or blocks.
            self.walk(ast, child);
        }
    }

    /// Every enum by name — used to find the single enum a `switch`'s referenced
    /// cases all belong to.
    pub(crate) fn enums(&self) -> impl Iterator<Item = (&String, &Vec<String>)> {
        self.enums.iter()
    }

    /// The declared return type of the function named `name`, if it is declared.
    pub(crate) fn func_return(&self, name: &str) -> Option<Type> {
        self.func_returns.get(name).copied()
    }
}

/// The case names declared directly under an `enum`, in source order. Only the
/// case-list `Block` is descended, so a nested type's cases are not mistaken for
/// this enum's.
fn collect_enum_cases(ast: &Ast, enum_decl: NodeId) -> Vec<String> {
    let mut cases = Vec::new();
    for child in child_ids(ast, enum_decl) {
        match ast.node(child).kind() {
            NodeKind::EnumCaseDecl => {
                if let Some(name) = ast.node(child).text() {
                    cases.push(name.to_string());
                }
            }
            NodeKind::Block => cases.extend(collect_enum_cases(ast, child)),
            _ => {}
        }
    }
    cases
}

/// The return type written on a `FuncDecl`'s `TypeRef`, or `Void` when none is
/// written or the annotation names an unmodelled type.
fn func_return_type(ast: &Ast, func_decl: NodeId) -> Type {
    child_ids(ast, func_decl)
        .into_iter()
        .find(|c| ast.node(*c).kind() == NodeKind::TypeRef)
        .and_then(|c| ast.node(c).text().and_then(parse_type_name))
        .unwrap_or(Type::Void)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_parser::parse;

    fn symbols_of(src: &str) -> Symbols {
        let ast = parse(src).expect("parse ok");
        Symbols::collect(&ast)
    }

    #[test]
    fn enums_iterates_every_enum_in_source_order() {
        let symbols = symbols_of("enum A { case x\n case y }\nenum B { case z }");
        let mut found: Vec<(&String, &Vec<String>)> = symbols.enums().collect();
        found.sort_by(|a, b| a.0.cmp(b.0));
        assert_eq!(found[0].0, "A");
        assert_eq!(found[0].1, &vec!["x".to_string(), "y".to_string()]);
        assert_eq!(found[1].0, "B");
        assert_eq!(found[1].1, &vec!["z".to_string()]);
    }

    #[test]
    fn collects_nested_enums() {
        let symbols = symbols_of("struct Outer { enum Inner { case a\n case b } }");
        let inner: Vec<(&String, &Vec<String>)> = symbols.enums().collect();
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].0, "Inner");
        assert_eq!(inner[0].1, &vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn records_declared_func_return_type() {
        let symbols = symbols_of("func answer() -> Int { return 42 }");
        assert_eq!(symbols.func_return("answer"), Some(Type::Int));
    }

    #[test]
    fn func_without_return_type_is_void() {
        let symbols = symbols_of("func greet() { }");
        assert_eq!(symbols.func_return("greet"), Some(Type::Void));
    }

    #[test]
    fn unknown_func_has_no_return_type() {
        let symbols = symbols_of("func greet() { }");
        assert_eq!(symbols.func_return("missing"), None);
    }
}
