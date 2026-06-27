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

/// One static method declared by a `@resultBuilder` type, recorded as the facts
/// the transform selects on: its name, the first parameter's argument label
/// (`first` in `buildEither(first:)`), and its parameter count.
///
/// `first_label` and `arity` are recorded now for the label/overload-based
/// selection later slices need (`buildEither`, partial-block folds); the walking
/// skeleton only reads `name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuilderMethod {
    pub(crate) name: String,
    #[allow(dead_code)] // recorded for label-based selection; not read yet.
    pub(crate) first_label: Option<String>,
    pub(crate) arity: usize,
    /// The written type of the first parameter (`String` in
    /// `buildExpression(_ v: String)`), used to reason about type-only
    /// overloads.
    pub(crate) first_param_type: Option<String>,
}

/// The static-method set declared by one `@resultBuilder` type — the menu the
/// result-builder transform selects build methods from (§4.1 of the plan).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct BuilderMethods {
    methods: Vec<BuilderMethod>,
}

impl BuilderMethods {
    /// Whether the builder declares any method named `name`.
    pub(crate) fn has(&self, name: &str) -> bool {
        self.methods.iter().any(|m| m.name == name)
    }

    /// Whether the builder declares a method `name` taking exactly `arity`
    /// parameters. Used to tell the `buildPartialBlock(first:)` (arity 1) and
    /// `buildPartialBlock(accumulated:next:)` (arity 2) overloads apart.
    pub(crate) fn has_arity(&self, name: &str, arity: usize) -> bool {
        self.methods
            .iter()
            .any(|m| m.name == name && m.arity == arity)
    }

    /// Every declared static method, in source order.
    #[allow(dead_code)] // read by later selection slices; exercised by tests now.
    pub(crate) fn methods(&self) -> &[BuilderMethod] {
        &self.methods
    }
}

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
    /// Each `@resultBuilder` type by name, mapped to its declared static
    /// build-method set. Drives the result-builder transform's method
    /// selection.
    result_builders: HashMap<String, BuilderMethods>,
    /// Each function by name, mapped to the `(parameter index, attribute name)`
    /// pairs for parameters carrying a custom attribute. Filtered to builder
    /// attributes by [`Symbols::func_builder_params`] (a custom attribute is
    /// only known to be a builder once all `@resultBuilder` types are walked).
    func_param_attrs: HashMap<String, Vec<(usize, String)>>,
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
            // A `@resultBuilder` type (of any nominal kind) records its static
            // build-method set. Checked independently of the kind-specific arms
            // below so an `@resultBuilder enum` is still recognized.
            if matches!(
                ast.node(child).kind(),
                NodeKind::StructDecl
                    | NodeKind::EnumDecl
                    | NodeKind::ClassDecl
                    | NodeKind::ActorDecl
            ) && has_result_builder_attr(ast, child)
            {
                if let Some(name) = ast.node(child).text() {
                    self.result_builders
                        .insert(name.to_string(), collect_builder_methods(ast, child));
                }
            }
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
                        let attrs = collect_param_attrs(ast, child);
                        if !attrs.is_empty() {
                            self.func_param_attrs.insert(name.to_string(), attrs);
                        }
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

    /// The static build-method set of the `@resultBuilder` type named `name`, if
    /// `name` names a result builder.
    pub(crate) fn result_builder(&self, name: &str) -> Option<&BuilderMethods> {
        self.result_builders.get(name)
    }

    /// The `(parameter index, builder name)` pairs for parameters of the
    /// function `name` annotated with a `@resultBuilder` attribute — the
    /// contextual builders applied to a closure-literal argument at a call site.
    pub(crate) fn func_builder_params(&self, name: &str) -> Vec<(usize, String)> {
        self.func_param_attrs
            .get(name)
            .map(|attrs| {
                attrs
                    .iter()
                    .filter(|(_, attr)| self.result_builders.contains_key(attr))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The `(parameter index, attribute name)` pairs for a function's parameters
/// that carry a custom `@Attr`. Indexed over `Param` children only.
fn collect_param_attrs(ast: &Ast, func: NodeId) -> Vec<(usize, String)> {
    let mut attrs = Vec::new();
    let mut index = 0;
    for child in child_ids(ast, func) {
        if ast.node(child).kind() != NodeKind::Param {
            continue;
        }
        for attr in child_ids(ast, child) {
            if ast.node(attr).kind() == NodeKind::Attribute {
                if let Some(name) = ast.node(attr).text() {
                    attrs.push((index, name.to_string()));
                }
            }
        }
        index += 1;
    }
    attrs
}

/// Whether a type declaration carries a `@resultBuilder` attribute.
fn has_result_builder_attr(ast: &Ast, decl: NodeId) -> bool {
    ast.node(decl)
        .children()
        .any(|c| c.kind() == NodeKind::Attribute && c.text() == Some("resultBuilder"))
}

/// Collect the declared `static` build methods of a `@resultBuilder` type. Both
/// direct member `FuncDecl`s and those nested in a member `Block` are scanned,
/// so the shape the parser produces for each nominal kind is handled.
fn collect_builder_methods(ast: &Ast, decl: NodeId) -> BuilderMethods {
    let mut methods = Vec::new();
    collect_builder_methods_into(ast, decl, &mut methods);
    BuilderMethods { methods }
}

fn collect_builder_methods_into(ast: &Ast, parent: NodeId, methods: &mut Vec<BuilderMethod>) {
    for child in child_ids(ast, parent) {
        match ast.node(child).kind() {
            NodeKind::FuncDecl => {
                let is_static = ast.node(child).modifiers().iter().any(|m| m == "static");
                if !is_static {
                    continue;
                }
                if let Some(name) = ast.node(child).text() {
                    let params: Vec<NodeId> = child_ids(ast, child)
                        .into_iter()
                        .filter(|c| ast.node(*c).kind() == NodeKind::Param)
                        .collect();
                    let first_label = params
                        .first()
                        .and_then(|p| ast.node(*p).arg_label().map(str::to_string));
                    let first_param_type = params.first().and_then(|p| {
                        child_ids(ast, *p)
                            .into_iter()
                            .find(|c| ast.node(*c).kind() == NodeKind::TypeRef)
                            .and_then(|c| ast.node(c).text().map(str::to_string))
                    });
                    methods.push(BuilderMethod {
                        name: name.to_string(),
                        first_label,
                        arity: params.len(),
                        first_param_type,
                    });
                }
            }
            NodeKind::Block => collect_builder_methods_into(ast, child, methods),
            _ => {}
        }
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

    #[test]
    fn records_result_builder_static_methods() {
        let symbols = symbols_of(
            "@resultBuilder\nstruct B {\n\
             static func buildExpression(_ v: String) -> String { v }\n\
             static func buildBlock(_ parts: String...) -> String { \"\" }\n}",
        );
        let b = symbols.result_builder("B").expect("B is a result builder");
        assert!(b.has("buildExpression"));
        assert!(b.has("buildBlock"));
        assert!(!b.has("buildOptional"));
        let block = b.methods().iter().find(|m| m.name == "buildBlock").unwrap();
        assert_eq!(block.arity, 1);
    }

    #[test]
    fn non_static_methods_are_not_builder_methods() {
        let symbols = symbols_of(
            "@resultBuilder\nstruct B {\n\
             func buildBlock(_ v: String) -> String { v }\n}",
        );
        let b = symbols.result_builder("B").unwrap();
        assert!(
            !b.has("buildBlock"),
            "instance methods are not build methods"
        );
    }

    #[test]
    fn func_builder_params_reports_builder_annotated_parameters() {
        let symbols = symbols_of(
            "@resultBuilder\nstruct SB {\n static func buildBlock(_ p: String...) -> String { \"\" } }\n\
             func wrap(_ x: Int, @SB _ content: () -> String) -> String { content() }",
        );
        let params = symbols.func_builder_params("wrap");
        assert_eq!(params, vec![(1, "SB".to_string())]);
    }

    #[test]
    fn func_builder_params_ignores_non_builder_attributes() {
        let symbols = symbols_of("func f(@objcMembers _ x: Int) { }");
        assert!(symbols.func_builder_params("f").is_empty());
    }

    #[test]
    fn a_plain_type_is_not_a_result_builder() {
        let symbols = symbols_of("struct B { static func buildBlock() -> Int { 0 } }");
        assert!(symbols.result_builder("B").is_none());
    }
}
