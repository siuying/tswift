//! The analysis pass pipeline.
//!
//! Semantic analysis is an ordered list of [`Pass`]es over one mutable [`Ast`],
//! sharing a single [`Symbols`] registry collected up front. Each pass reads
//! declarations through `symbols`, may rewrite the tree, and returns
//! diagnostics. Passes run in list order, so ordering constraints — for example
//! an AST→AST rewrite that must precede type annotation — are stated here in
//! [`pipeline`] rather than buried in the call order of one fused function.

use tswift_ast::Ast;

use crate::builder_transform::BuilderTransform;
use crate::symbols::Symbols;
use crate::{annotate, Diagnostic};

/// One analysis pass over the shared AST and declaration registry.
///
/// This is the single interface every stage of analysis implements, so passes
/// can be ordered, swapped, and tested uniformly. A pass may mutate `ast` (a
/// rewrite) or leave it unchanged (a pure check); either way it reports
/// diagnostics in source order.
pub(crate) trait Pass {
    // NOTE: implemented by passes in sibling modules (e.g. `BuilderTransform`).

    /// Run the pass over `ast`, reading declarations from `symbols`, and return
    /// any diagnostics in source order.
    fn run(&self, ast: &mut Ast, symbols: &Symbols) -> Vec<Diagnostic>;
}

/// Name resolution + type annotation + semantic diagnostics.
///
/// Wraps the existing resolver ([`annotate`]) as a pass. It reads declarations
/// but does not rewrite the tree.
struct Annotate;

impl Pass for Annotate {
    fn run(&self, ast: &mut Ast, symbols: &Symbols) -> Vec<Diagnostic> {
        annotate(ast, symbols)
    }
}

/// The ordered analysis pipeline.
///
/// AST→AST transforms (such as the result-builder rewrite) slot in *before*
/// [`Annotate`], because annotation must see the rewritten tree.
fn pipeline() -> Vec<Box<dyn Pass>> {
    vec![Box::new(BuilderTransform), Box::new(Annotate)]
}

/// Run every analysis pass over `ast` in pipeline order, returning the
/// diagnostics from each pass concatenated in that order.
///
/// The [`Symbols`] registry is collected once, before any pass, and shared by
/// reference with every pass.
pub fn analyze(ast: &mut Ast) -> Vec<Diagnostic> {
    let symbols = Symbols::collect(ast);
    let mut diagnostics = Vec::new();
    for pass in pipeline() {
        diagnostics.extend(pass.run(ast, &symbols));
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_parser::parse;

    /// A no-op pass: exercises the [`Pass`] interface without touching the tree.
    struct Noop;
    impl Pass for Noop {
        fn run(&self, _ast: &mut Ast, _symbols: &Symbols) -> Vec<Diagnostic> {
            Vec::new()
        }
    }

    #[test]
    fn annotate_pass_runs_through_the_interface() {
        let mut ast = parse("let x = 1 + 2").expect("parse ok");
        let symbols = Symbols::collect(&ast);
        let pass = Annotate;
        let diags = pass.run(&mut ast, &symbols);
        assert!(diags.is_empty(), "{diags:?}");
        // The annotate pass recorded a type on the initializer expression.
        let decl = ast.node(ast.root()).children().next().unwrap();
        assert_eq!(decl.children().last().unwrap().type_name(), Some("Int"));
    }

    #[test]
    fn noop_pass_reports_nothing_and_leaves_tree_untouched() {
        let mut ast = parse("let x = 1").expect("parse ok");
        let symbols = Symbols::collect(&ast);
        let before = ast.node(ast.root()).children().count();
        let diags = Noop.run(&mut ast, &symbols);
        assert!(diags.is_empty());
        assert_eq!(ast.node(ast.root()).children().count(), before);
    }

    #[test]
    fn analyze_drives_the_pipeline_and_surfaces_diagnostics() {
        let mut ast = parse(r#"let x: Int = "oops""#).expect("parse ok");
        let diags = analyze(&mut ast);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("cannot convert"), "{diags:?}");
    }
}
