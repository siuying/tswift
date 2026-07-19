//! Expanding `@Test(arguments:)` into one case per argument combination.
//!
//! The `arguments:` collections are inline expressions in the `@Test`
//! attribute (plan §2.1). We expand structurally, without evaluating: each
//! element node becomes a call argument, rendered back to source for the
//! driver (the interpreter is argument-label-lenient, so positional calls
//! suffice). Supported collection shapes:
//!
//! - a single array literal `[a, b, c]` → one case per element;
//! - multiple array literals `[a, b], [c, d]` → the cartesian product;
//! - `zip(a, b)` → element-wise pairing (shorter collection wins).
//!
//! Any other shape (a named collection, a range, a function call) is left
//! unexpanded — the test is reported once and its unbound parameters are the
//! user's problem, exactly as an un-parameterized `@Test` with parameters.

use tswift_frontend::{Node, NodeKind};

use crate::render;

/// Outcome of examining a `@Test`'s `arguments:` attribute.
pub enum Expansion {
    /// No `arguments:` label — an ordinary, non-parameterized test.
    None,
    /// Parameterized with these per-case argument node rows (one node per
    /// function parameter). May be empty (`arguments: []`, an empty `zip`, or
    /// an empty cartesian factor) — the caller decides how to report that.
    Cases(Vec<Vec<Node<'static>>>),
    /// Has `arguments:`, but the expression shape isn't a collection literal
    /// or `zip(...)` we can expand structurally. Carries a rendered spelling
    /// of the unsupported collection(s) for the error message.
    Unsupported(String),
}

/// Expand the `@Test(arguments:)` of `func_decl`. See [`Expansion`].
pub fn expand(func_decl: &Node<'static>) -> Expansion {
    let Some(attr) = func_decl
        .children()
        .find(|c| c.kind() == NodeKind::Attribute && c.text().as_deref() == Some("Test"))
    else {
        return Expansion::None;
    };
    let children: Vec<Node<'static>> = attr.children().collect();
    let Some(start) = children
        .iter()
        .position(|c| c.arg_label().as_deref() == Some("arguments"))
    else {
        return Expansion::None;
    };
    let collections = &children[start..];

    if collections.len() == 1 {
        if let Some(zipped) = zip_cases(&collections[0]) {
            return Expansion::Cases(zipped);
        }
    }
    let lists: Option<Vec<Vec<Node<'static>>>> = collections.iter().map(elements).collect();
    match lists {
        Some(lists) => Expansion::Cases(cartesian(&lists)),
        None => {
            let spelling: Vec<String> = collections.iter().map(render::expr).collect();
            Expansion::Unsupported(spelling.join(", "))
        }
    }
}

/// A human-readable call signature for the test function (`div(x:)`), used as
/// the base of a parameterized case's display label.
pub fn signature(func_decl: &Node<'_>, name: &str) -> String {
    let labels: String = func_decl
        .children()
        .filter(|c| c.kind() == NodeKind::Param)
        .map(|p| {
            let info = p.param_info();
            let label = info.label.unwrap_or(info.name);
            format!("{label}:")
        })
        .collect();
    format!("{name}({labels})")
}

/// The disambiguated ` - <args>` id/label suffix for each row of expanded
/// arguments, index-aligned with `rows`: duplicate-argument rows get a
/// trailing ` (#n)` occurrence tag so no two suffixes collide. Shared by the
/// runner ([`crate::plan_case`]'s per-case id/label) and
/// [`crate::descriptor::list_tests`]'s listed case ids, so both compute the
/// exact same selectable id a host passes back in `RunOptions::ids`.
pub fn case_id_suffixes(rows: &[Vec<Node<'static>>]) -> Vec<String> {
    let rendered: Vec<String> = rows
        .iter()
        .map(|row| row.iter().map(render::expr).collect::<Vec<_>>().join(", "))
        .collect();
    let mut total_occurrences: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    for args in &rendered {
        *total_occurrences.entry(args.as_str()).or_insert(0) += 1;
    }
    let mut seen_so_far: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    rendered
        .iter()
        .map(|args| {
            let suffix = if total_occurrences[args.as_str()] > 1 {
                let n = seen_so_far.entry(args.as_str()).or_insert(0);
                *n += 1;
                format!(" (#{n})")
            } else {
                String::new()
            };
            format!(" - {args}{suffix}")
        })
        .collect()
}

/// The element nodes of an array-literal collection, else `None`.
fn elements(collection: &Node<'static>) -> Option<Vec<Node<'static>>> {
    if collection.kind() != NodeKind::ArrayLiteral {
        return None;
    }
    Some(collection.children().collect())
}

/// Element-wise pairing for `zip(a, b, …)`: each case takes the i-th element of
/// every argument collection, up to the shortest. `None` when `node` is not a
/// `zip(...)` call over array literals.
fn zip_cases(node: &Node<'static>) -> Option<Vec<Vec<Node<'static>>>> {
    if node.kind() != NodeKind::CallExpr {
        return None;
    }
    let mut children = node.children();
    let callee = children.next()?;
    if callee.kind() != NodeKind::IdentExpr || callee.text().as_deref() != Some("zip") {
        return None;
    }
    let lists: Vec<Vec<Node<'static>>> = children.map(|c| elements(&c)).collect::<Option<_>>()?;
    let rows = lists.iter().map(Vec::len).min().unwrap_or(0);
    Some(
        (0..rows)
            .map(|i| lists.iter().map(|l| l[i]).collect())
            .collect(),
    )
}

/// The cartesian product of the per-parameter element lists, with the first
/// collection varying slowest (`[1,2] × [a,b]` → `1a, 1b, 2a, 2b`).
fn cartesian(lists: &[Vec<Node<'static>>]) -> Vec<Vec<Node<'static>>> {
    let mut rows: Vec<Vec<Node<'static>>> = vec![Vec::new()];
    for list in lists {
        let mut next = Vec::with_capacity(rows.len() * list.len());
        for prefix in &rows {
            for elem in list {
                let mut row = prefix.clone();
                row.push(*elem);
                next.push(row);
            }
        }
        rows = next;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_frontend::Analysis;

    fn func(src: &str) -> Node<'static> {
        let analysis: &'static Analysis =
            Box::leak(Box::new(Analysis::analyze(src, "t.swift").unwrap()));
        analysis.root().children().next().unwrap()
    }

    fn cases_of(decl: &Node<'static>) -> Vec<Vec<Node<'static>>> {
        match expand(decl) {
            Expansion::Cases(rows) => rows,
            _ => panic!("expected Expansion::Cases"),
        }
    }

    #[test]
    fn single_array_expands_per_element() {
        let cases = cases_of(&func("@Test(arguments: [1, 2, 3]) func p(x: Int) {}\n"));
        assert_eq!(cases.len(), 3);
        assert!(cases.iter().all(|row| row.len() == 1));
    }

    #[test]
    fn two_arrays_are_cartesian() {
        let cases = cases_of(&func(
            "@Test(arguments: [1, 2], [3, 4]) func p(x: Int, y: Int) {}\n",
        ));
        assert_eq!(cases.len(), 4);
        assert!(cases.iter().all(|row| row.len() == 2));
    }

    #[test]
    fn zip_pairs_element_wise() {
        let cases = cases_of(&func(
            "@Test(arguments: zip([1, 2], [3, 4])) func p(a: Int, b: Int) {}\n",
        ));
        assert_eq!(cases.len(), 2);
        assert!(cases.iter().all(|row| row.len() == 2));
    }

    #[test]
    fn non_parameterized_is_none() {
        assert!(matches!(
            expand(&func("@Test func p() {}\n")),
            Expansion::None
        ));
    }

    #[test]
    fn empty_array_literal_expands_to_zero_cases() {
        let cases = cases_of(&func("@Test(arguments: []) func p(x: Int) {}\n"));
        assert!(cases.is_empty());
    }

    #[test]
    fn empty_zip_factor_expands_to_zero_cases() {
        let cases = cases_of(&func(
            "@Test(arguments: zip([1, 2], [])) func p(a: Int, b: Int) {}\n",
        ));
        assert!(cases.is_empty());
    }

    #[test]
    fn empty_cartesian_factor_expands_to_zero_cases() {
        let cases = cases_of(&func(
            "@Test(arguments: [1, 2], []) func p(x: Int, y: Int) {}\n",
        ));
        assert!(cases.is_empty());
    }

    #[test]
    fn non_collection_arguments_is_unsupported() {
        let outcome = expand(&func(
            "@Test(arguments: someNamedCollection) func p(x: Int) {}\n",
        ));
        assert!(matches!(outcome, Expansion::Unsupported(_)));
    }

    #[test]
    fn case_id_suffixes_disambiguate_duplicate_args() {
        let rows = cases_of(&func("@Test(arguments: [1, 1]) func p(x: Int) {}\n"));
        let suffixes = case_id_suffixes(&rows);
        assert_eq!(
            suffixes,
            vec![" - 1 (#1)".to_string(), " - 1 (#2)".to_string()]
        );
    }

    #[test]
    fn case_id_suffixes_no_suffix_when_unique() {
        let rows = cases_of(&func("@Test(arguments: [1, 2]) func p(x: Int) {}\n"));
        let suffixes = case_id_suffixes(&rows);
        assert_eq!(suffixes, vec![" - 1".to_string(), " - 2".to_string()]);
    }

    #[test]
    fn signature_shows_parameter_labels() {
        assert_eq!(
            signature(&func("@Test func div(x: Int) {}\n"), "div"),
            "div(x:)"
        );
    }
}
