//! Discovery: walk the typed AST for `@Test` free functions and `@Test`
//! methods in types (implicit suites). No macro expansion, no `@main` — just a
//! structural walk of the program root (plan §2.3).

use tswift_frontend::{Node, NodeKind};

/// One discovered test: a free `@Test` function or a `@Test` method on a type.
#[derive(Clone)]
pub struct TestCase {
    /// The suite type this test belongs to, or `None` for a free test.
    pub suite_type: Option<String>,
    /// `@Suite("…")` display name of the owning type, if any.
    pub suite_display: Option<String>,
    /// The test function's declared name (no parentheses).
    pub func_name: String,
    /// `@Test("…")` display name, if given.
    pub display_name: Option<String>,
    /// The `FuncDecl` node.
    pub node: Node<'static>,
    /// 1-based combined-source line of the declaration (for stable ordering and
    /// location remapping).
    pub line: u32,
}

impl TestCase {
    /// The fully-qualified id: `"free()"` or `"MathSuite/pass()"`.
    pub fn id(&self) -> String {
        match &self.suite_type {
            Some(suite) => format!("{suite}/{}()", self.func_name),
            None => format!("{}()", self.func_name),
        }
    }

    /// Whether `needle` matches this test by id or display name (case-sensitive
    /// substring — the v1 `--filter` contract, plan §4.2).
    pub fn matches_filter(&self, needle: &str) -> bool {
        self.id().contains(needle)
            || self
                .display_name
                .as_deref()
                .is_some_and(|d| d.contains(needle))
            || self
                .suite_type
                .as_deref()
                .is_some_and(|s| s.contains(needle))
    }
}

/// Discover every test under `root`, in stable declaration order (by line).
pub fn discover(root: Node<'static>) -> Vec<TestCase> {
    let mut cases = Vec::new();
    for decl in root.children() {
        match decl.kind() {
            NodeKind::FuncDecl if has_attribute(&decl, "Test") => {
                cases.push(TestCase {
                    suite_type: None,
                    suite_display: None,
                    func_name: decl.decl_name().unwrap_or_default(),
                    display_name: attribute_display_name(&decl, "Test"),
                    node: decl,
                    line: decl.line(),
                });
            }
            NodeKind::StructDecl | NodeKind::ClassDecl | NodeKind::ActorDecl => {
                collect_suite(&decl, &mut cases);
            }
            _ => {}
        }
    }
    cases.sort_by_key(|c| c.line);
    cases
}

/// Collect `@Test` methods of a type. A type with any `@Test` method is a suite
/// even without `@Suite` (implicit suites, matching Apple).
fn collect_suite(type_decl: &Node<'static>, cases: &mut Vec<TestCase>) {
    let suite_type = type_decl.decl_name();
    let Some(suite_type) = suite_type else {
        return;
    };
    let suite_display = attribute_display_name(type_decl, "Suite");
    for member in type_decl.children() {
        if member.kind() == NodeKind::FuncDecl && has_attribute(&member, "Test") {
            cases.push(TestCase {
                suite_type: Some(suite_type.clone()),
                suite_display: suite_display.clone(),
                func_name: member.decl_name().unwrap_or_default(),
                display_name: attribute_display_name(&member, "Test"),
                node: member,
                line: member.line(),
            });
        }
    }
}

/// Whether `decl` carries an attribute named `name` (`@Test`, `@Suite`; the
/// frontend strips the leading `@`).
fn has_attribute(decl: &Node<'_>, name: &str) -> bool {
    attribute(decl, name).is_some()
}

fn attribute<'a>(decl: &Node<'a>, name: &str) -> Option<Node<'a>> {
    decl.children()
        .find(|c| c.kind() == NodeKind::Attribute && c.text().as_deref() == Some(name))
}

/// The first unlabelled string argument of attribute `name` (`@Test("x")` →
/// `Some("x")`), unquoted.
fn attribute_display_name(decl: &Node<'_>, name: &str) -> Option<String> {
    let attr = attribute(decl, name)?;
    let literal = attr
        .children()
        .find(|c| c.kind() == NodeKind::StringLiteral)?;
    Some(unquote(&literal.text()?))
}

/// Strip surrounding double quotes from a string-literal spelling.
fn unquote(spelling: &str) -> String {
    spelling
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(spelling)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_frontend::Analysis;

    fn discover_src(src: &str) -> Vec<TestCase> {
        let analysis = Analysis::analyze(src, "t.swift").unwrap();
        let analysis: &'static Analysis = Box::leak(Box::new(analysis));
        discover(analysis.root())
    }

    #[test]
    fn discovers_free_test_with_display_name() {
        let cases = discover_src("@Test(\"adds\")\nfunc addition() {}\n");
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].id(), "addition()");
        assert_eq!(cases[0].display_name.as_deref(), Some("adds"));
        assert!(cases[0].suite_type.is_none());
    }

    #[test]
    fn discovers_implicit_suite_methods() {
        let src = "struct MathSuite {\n  @Test func pass() {}\n  func helper() {}\n}\n";
        let cases = discover_src(src);
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].id(), "MathSuite/pass()");
    }

    #[test]
    fn discovery_is_stable_by_line() {
        let src = "@Test func b() {}\n@Test func a() {}\n";
        let cases = discover_src(src);
        let ids: Vec<String> = cases.iter().map(|c| c.id()).collect();
        assert_eq!(ids, vec!["b()", "a()"]);
    }

    #[test]
    fn captures_suite_display_name() {
        let src = "@Suite(\"My Suite\") struct S {\n  @Test func t() {}\n}\n";
        let cases = discover_src(src);
        assert_eq!(cases[0].suite_display.as_deref(), Some("My Suite"));
    }
}
