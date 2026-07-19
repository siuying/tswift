//! Parsing `@Test`/`@Suite` trait arguments into a skip decision.
//!
//! Traits are written as leading-dot calls in the attribute argument list
//! (`.disabled("reason")`, `.enabled(if: cond)`), parsed structurally as a
//! `CallExpr` whose callee is a `MemberExpr` (plan §1.2, R10). We model only
//! the two skip-affecting traits; every other trait (`.tags`, `.bug`,
//! `.serialized`, …) is recognised-but-ignored so it never breaks discovery.

use std::time::Duration;

use tswift_frontend::{Node, NodeKind};

/// A trait that can skip a test.
#[derive(Clone)]
pub enum Trait {
    /// `.disabled("reason")` — always skip, carrying an optional reason.
    Disabled(Option<String>),
    /// `.enabled(if: cond)` — skip unless `cond` evaluates to `true`; the
    /// condition node is evaluated once at run start against the loaded
    /// program. A suite-level `@Suite(.enabled(if: cond))` is inherited by
    /// every `@Test` member (`discover::collect_suite`), so its condition is
    /// re-evaluated once *per member* — a side-effecting condition runs
    /// multiple times, once per test in the suite, not once for the suite as
    /// a whole.
    EnabledIf(Node<'static>),
    /// `.tags(.fast, .slow)` — associate the test with one or more tag names.
    /// Tag identity is by unqualified name (`.fast` → `"fast"`, `Tag.custom` →
    /// `"custom"`, and `MyTags.custom` also → `"custom"`); the runtime has no
    /// `Tag` value, so this is a structural read of the attribute (plan §1.2)
    /// and differently-typed tags sharing a final component collide — see
    /// [`tag_name`]. Inherited by suite members like every trait.
    Tags(Vec<String>),
    /// `.bug("url-or-id"[, "title"])` — a report-only annotation surfaced on
    /// failure. Carries the first argument's spelling (URL or identifier).
    Bug(String),
    /// `.timeLimit(.minutes(n))` — a *soft* limit: the runner measures the
    /// test's duration and records an issue when it is exceeded, but never
    /// hard-kills the test (tswift has no host timer policy; plan §1.2).
    TimeLimit(Duration),
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
        Some("tags") => Some(Trait::Tags(children.filter_map(|n| tag_name(&n)).collect())),
        Some("bug") => children.next().map(|n| Trait::Bug(bug_reference(&n))),
        Some("timeLimit") => children
            .next()
            .and_then(|n| time_limit(&n))
            .map(Trait::TimeLimit),
        _ => None,
    }
}

/// The tag name of a `.tags(...)` argument: a leading-dot member (`.fast`) or a
/// `Tag.custom` reference both reduce to their final component (`"fast"`,
/// `"custom"`).
///
/// **Divergence from real Swift Testing**: Apple's `Tag` has true nominal
/// identity (`Tag.fast` and `MyTags.fast` are distinct static members even
/// with the same name). tswift has no `Tag` value at all — tags are a
/// structural read of the attribute's unqualified final component, so
/// `Tag.fast` and `MyTags.fast` collide into the same `"fast"` name and
/// `--filter tag:fast` matches both. Documented rather than silently
/// diverging; see `website/src/pages/status/testing.mdx`.
fn tag_name(node: &Node<'_>) -> Option<String> {
    if node.kind() != NodeKind::MemberExpr {
        return None;
    }
    node.text().map(|t| t.to_string())
}

/// The reference string of a `.bug(...)` argument — a string literal's contents
/// (`"url"`), or the source spelling of a non-string first argument (an id).
fn bug_reference(node: &Node<'_>) -> String {
    if let Some(s) = string_value(node) {
        return s;
    }
    node.text().map(|t| t.to_string()).unwrap_or_default()
}

/// Parse a `.timeLimit(...)` duration argument. Only the `.minutes(n)` and
/// `.seconds(n)` unit forms are recognised; anything else yields `None` and
/// the trait is ignored.
///
/// **Divergence from real Swift Testing**: Apple's `timeLimit` trait only
/// accepts a `.minutes(n)` `Duration` and rounds sub-minute components up to
/// the next whole minute — there is no `.seconds` overload. tswift also
/// accepts `.seconds(n)` and applies it exactly (no rounding), which is more
/// convenient for fast test suites and documented here rather than silently
/// diverging; see `website/src/pages/status/testing.mdx`.
fn time_limit(node: &Node<'_>) -> Option<Duration> {
    if node.kind() != NodeKind::CallExpr {
        return None;
    }
    let mut children = node.children();
    let callee = children.next()?;
    if callee.kind() != NodeKind::MemberExpr || callee.first_child().is_some() {
        return None;
    }
    let amount: u64 = children.next()?.text()?.parse().ok()?;
    match callee.text().as_deref() {
        Some("minutes") => Some(Duration::from_secs(amount * 60)),
        Some("seconds") => Some(Duration::from_secs(amount)),
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
        let analysis: &'static Analysis =
            Box::leak(Box::new(Analysis::analyze(src, "t.swift").unwrap()));
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
    fn parses_tags() {
        let traits = traits_src("@Test(.tags(.fast, .slow)) func t() {}\n");
        assert_eq!(traits.len(), 1);
        assert!(
            matches!(&traits[0], Trait::Tags(names) if names == &["fast".to_string(), "slow".to_string()])
        );
    }

    #[test]
    fn parses_bug_reference() {
        let traits = traits_src("@Test(.bug(\"https://example.com/42\")) func t() {}\n");
        assert!(matches!(&traits[0], Trait::Bug(r) if r == "https://example.com/42"));
    }

    #[test]
    fn parses_time_limit_minutes() {
        let traits = traits_src("@Test(.timeLimit(.minutes(2))) func t() {}\n");
        assert!(
            matches!(&traits[0], Trait::TimeLimit(d) if *d == std::time::Duration::from_secs(120))
        );
    }

    #[test]
    fn tag_identity_collides_by_unqualified_name() {
        // Documented divergence: `Tag.fast` and `MyTags.fast` have distinct
        // nominal identity in real Swift Testing but reduce to the same
        // `"fast"` name here, since tswift has no `Tag` value.
        let traits = traits_src("@Test(.tags(Tag.fast, MyTags.fast)) func t() {}\n");
        assert!(
            matches!(&traits[0], Trait::Tags(names) if names == &["fast".to_string(), "fast".to_string()])
        );
    }

    #[test]
    fn parses_time_limit_seconds_without_rounding() {
        // Divergence from real Swift Testing (which has no `.seconds`
        // overload and rounds `.minutes` sub-components up): tswift accepts
        // `.seconds(n)` and applies it exactly, with no rounding up to a
        // whole minute.
        let traits = traits_src("@Test(.timeLimit(.seconds(90))) func t() {}\n");
        assert!(
            matches!(&traits[0], Trait::TimeLimit(d) if *d == std::time::Duration::from_secs(90))
        );
    }

    #[test]
    fn ignores_truly_unknown_trait() {
        let traits = traits_src("@Test(.serialized) func t() {}\n");
        assert!(traits.is_empty());
    }
}
