//! Parsing `@Test`/`@Suite` trait arguments into a skip decision.
//!
//! Traits are written as leading-dot calls in the attribute argument list
//! (`.disabled("reason")`, `.enabled(if: cond)`), parsed structurally as a
//! `CallExpr` whose callee is a `MemberExpr` (plan §1.2, R10). We model only
//! the two skip-affecting traits; every other trait (`.tags`, `.bug`,
//! `.serialized`, …) is recognised-but-ignored so it never breaks discovery.

use tswift_frontend::{Node, NodeKind};

/// A trait that can skip a test.
#[derive(Clone)]
pub enum Trait {
    /// `.disabled("reason")` — always skip, carrying an optional reason.
    Disabled(Option<String>),
    /// `.enabled(if: cond)` — skip unless `cond` evaluates to `true`; the
    /// condition node is evaluated once at run start against the loaded program.
    EnabledIf(Node<'static>),
}

/// Extract the skip-affecting traits from attribute `name` (`Test`/`Suite`) on
/// `decl`, in source order.
pub fn traits_of(decl: &Node<'static>, name: &str) -> Vec<Trait> {
    let Some(attr) = decl
        .children()
        .find(|c| c.kind() == NodeKind::Attribute && c.text().as_deref() == Some(name))
    else {
        return Vec::new();
    };
    attr.children().filter_map(parse_trait).collect()
}

/// Parse one attribute argument into a [`Trait`], if it is a recognised
/// leading-dot trait call.
fn parse_trait(arg: Node<'static>) -> Option<Trait> {
    if arg.kind() != NodeKind::CallExpr {
        return None;
    }
    let mut children = arg.children();
    let callee = children.next()?;
    if callee.kind() != NodeKind::MemberExpr || callee.first_child().is_some() {
        // A trait is a *leading-dot* member (`.disabled`); a `Foo.bar()` call
        // (callee has a base) is not a trait.
        return None;
    }
    match callee.text().as_deref() {
        Some("disabled") => {
            let reason = children.next().and_then(|n| string_value(&n));
            Some(Trait::Disabled(reason))
        }
        Some("enabled") => children.next().map(Trait::EnabledIf),
        _ => None,
    }
}

/// The unquoted value of a string-literal node, if that is what `node` is.
fn string_value(node: &Node<'_>) -> Option<String> {
    if node.kind() != NodeKind::StringLiteral {
        return None;
    }
    let spelling = node.text()?;
    Some(
        spelling
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(&spelling)
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tswift_frontend::Analysis;

    fn traits_src(src: &str) -> Vec<Trait> {
        let analysis: &'static Analysis = Box::leak(Box::new(Analysis::analyze(src, "t.swift").unwrap()));
        let func = analysis.root().children().next().unwrap();
        traits_of(&func, "Test")
    }

    #[test]
    fn parses_disabled_reason() {
        let traits = traits_src("@Test(.disabled(\"flaky\")) func t() {}\n");
        assert_eq!(traits.len(), 1);
        assert!(matches!(&traits[0], Trait::Disabled(Some(r)) if r == "flaky"));
    }

    #[test]
    fn parses_enabled_if_condition() {
        let traits = traits_src("@Test(.enabled(if: 1 > 0)) func t() {}\n");
        assert_eq!(traits.len(), 1);
        assert!(matches!(&traits[0], Trait::EnabledIf(_)));
    }

    #[test]
    fn ignores_unknown_trait() {
        let traits = traits_src("@Test(.tags(.fast)) func t() {}\n");
        assert!(traits.is_empty());
    }
}
