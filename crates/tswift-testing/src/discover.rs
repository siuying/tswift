//! Discovery: walk the typed AST for `@Test` free functions and `@Test`
//! methods in types (implicit suites). No macro expansion, no `@main` — just a
//! structural walk of the program root (plan §2.3).

use tswift_frontend::{Node, NodeKind};

use crate::traits::{traits_of, Trait};

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
    /// The test's own traits followed by any inherited from its suite
    /// (suite-level traits apply to every member, plan §1.2).
    pub traits: Vec<Trait>,
}

impl TestCase {
    /// The fully-qualified id: `"free()"`, `"MathSuite/pass()"`, or a nested
    /// `"Outer/Inner/b()"`. The suite path is stored dot-joined for runtime
    /// construction (`Outer.Inner()`); the id shows it slash-separated.
    pub fn id(&self) -> String {
        match &self.suite_type {
            Some(suite) => format!("{}/{}()", suite.replace('.', "/"), self.func_name),
            None => format!("{}()", self.func_name),
        }
    }

    /// The human display label composing the owning suite's and the test's
    /// display names (`"Math Suite/adds two numbers"`), falling back to the
    /// func/type id when neither is set so a suite test never loses its
    /// qualifying type name.
    pub fn label_base(&self) -> String {
        if self.suite_display.is_none() && self.display_name.is_none() {
            return self.id();
        }
        let test_part = self
            .display_name
            .clone()
            .unwrap_or_else(|| format!("{}()", self.func_name));
        match &self.suite_display {
            Some(suite) => format!("{suite}/{test_part}"),
            None => test_part,
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
                let traits = traits_of(&decl, "Test");
                cases.push(TestCase {
                    suite_type: None,
                    suite_display: None,
                    func_name: decl.decl_name().unwrap_or_default(),
                    display_name: attribute_display_name(&decl, "Test"),
                    line: decl.line(),
                    node: decl,
                    traits,
                });
            }
            NodeKind::StructDecl | NodeKind::ClassDecl | NodeKind::ActorDecl => {
                collect_suite(&decl, None, &[], &mut cases);
            }
            _ => {}
        }
    }
    cases.sort_by_key(|c| c.line);
    cases
}

/// Collect `@Test` methods of a type, recursing into nested suite types. A type
/// with any `@Test` method is a suite even without `@Suite` (implicit suites,
/// matching Apple). `parent` is the dot-joined construction path of the
/// enclosing suite, so a nested `Inner` under `Outer` is constructed as
/// `Outer.Inner()`.
fn collect_suite(
    type_decl: &Node<'static>,
    parent: Option<&str>,
    inherited: &[Trait],
    cases: &mut Vec<TestCase>,
) {
    let Some(name) = type_decl.decl_name() else {
        return;
    };
    let suite_type = match parent {
        Some(prefix) => format!("{prefix}.{name}"),
        None => name,
    };
    let suite_display = attribute_display_name(type_decl, "Suite");
    // A nested suite inherits its enclosing suite's traits plus its own
    // `@Suite` traits; every `@Test` member inherits that combined set.
    let mut suite_traits = inherited.to_vec();
    suite_traits.extend(traits_of(type_decl, "Suite"));
    for member in type_decl.children() {
        match member.kind() {
            NodeKind::FuncDecl if has_attribute(&member, "Test") => {
                let mut traits = traits_of(&member, "Test");
                traits.extend(suite_traits.iter().cloned());
                cases.push(TestCase {
                    suite_type: Some(suite_type.clone()),
                    suite_display: suite_display.clone(),
                    func_name: member.decl_name().unwrap_or_default(),
                    display_name: attribute_display_name(&member, "Test"),
                    line: member.line(),
                    node: member,
                    traits,
                });
            }
            NodeKind::StructDecl | NodeKind::ClassDecl | NodeKind::ActorDecl => {
                collect_suite(&member, Some(&suite_type), &suite_traits, cases);
            }
            _ => {}
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
    fn discovers_nested_suite_with_dotted_path() {
        let src = "struct Outer {\n  struct Inner {\n    @Test func b() {}\n  }\n}\n";
        let cases = discover_src(src);
        assert_eq!(cases[0].id(), "Outer/Inner/b()");
        assert_eq!(cases[0].suite_type.as_deref(), Some("Outer.Inner"));
    }

    #[test]
    fn captures_suite_display_name() {
        let src = "@Suite(\"My Suite\") struct S {\n  @Test func t() {}\n}\n";
        let cases = discover_src(src);
        assert_eq!(cases[0].suite_display.as_deref(), Some("My Suite"));
    }
}
