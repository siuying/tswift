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

/// Expand the `@Test(arguments:)` of `func_decl` into per-case argument node
/// lists (one node per function parameter). `None` when the test is not
/// parameterized; an empty `Vec` when it is but no combination is produced.
pub fn expand(func_decl: &Node<'static>) -> Option<Vec<Vec<Node<'static>>>> {
    let attr = func_decl
        .children()
        .find(|c| c.kind() == NodeKind::Attribute && c.text().as_deref() == Some("Test"))?;
    let children: Vec<Node<'static>> = attr.children().collect();
    let start = children
        .iter()
        .position(|c| c.arg_label().as_deref() == Some("arguments"))?;
    let collections = &children[start..];

    if collections.len() == 1 {
        if let Some(zipped) = zip_cases(&collections[0]) {
            return Some(zipped);
        }
    }
    let lists: Option<Vec<Vec<Node<'static>>>> = collections.iter().map(elements).collect();
    Some(cartesian(&lists?))
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
            .map(|i| lists.iter().map(|l| l[i].clone()).collect())
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
                row.push(elem.clone());
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
        let analysis: &'static Analysis = Box::leak(Box::new(Analysis::analyze(src, "t.swift").unwrap()));
        analysis.root().children().next().unwrap()
    }

    #[test]
    fn single_array_expands_per_element() {
        let cases = expand(&func("@Test(arguments: [1, 2, 3]) func p(x: Int) {}\n")).unwrap();
        assert_eq!(cases.len(), 3);
        assert!(cases.iter().all(|row| row.len() == 1));
    }

    #[test]
    fn two_arrays_are_cartesian() {
        let cases =
            expand(&func("@Test(arguments: [1, 2], [3, 4]) func p(x: Int, y: Int) {}\n")).unwrap();
        assert_eq!(cases.len(), 4);
        assert!(cases.iter().all(|row| row.len() == 2));
    }

    #[test]
    fn zip_pairs_element_wise() {
        let cases = expand(&func(
            "@Test(arguments: zip([1, 2], [3, 4])) func p(a: Int, b: Int) {}\n",
        ))
        .unwrap();
        assert_eq!(cases.len(), 2);
        assert!(cases.iter().all(|row| row.len() == 2));
    }

    #[test]
    fn non_parameterized_is_none() {
        assert!(expand(&func("@Test func p() {}\n")).is_none());
    }

    #[test]
    fn signature_shows_parameter_labels() {
        assert_eq!(signature(&func("@Test func div(x: Int) {}\n"), "div"), "div(x:)");
    }
}
