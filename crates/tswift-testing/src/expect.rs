//! The `#expect` / `#require` freestanding-macro builtins.
//!
//! Registered on the interpreter via `register_macro` (same seam as
//! `#Predicate`), so they are not expanded to Swift AST — each handler
//! inspects the `CompilerDirective` node's operand, evaluates it, and records
//! against the current [`crate::session`] (plan §3).

use tswift_core::{ops, Arg, EvalError, StdContext, StdError, StdResult, SwiftValue};
use tswift_frontend::{Node, NodeKind};

use crate::render;
use crate::session;

/// The result of checking one expectation, carrying a preformatted failure
/// detail so callers never re-evaluate operands.
enum Outcome {
    Passed,
    Failed(String),
}

/// `#expect(expr)` — soft check. Records an issue on failure but lets the test
/// body continue. A non-`Bool` operand or use outside a test is a hard error.
pub fn expect_macro(ctx: &mut dyn StdContext, node: &Node<'static>) -> StdResult {
    if !session::is_active() {
        return Err(trap("#expect used outside a test"));
    }
    if let Some((expected, closure)) = throws_operands(node) {
        return match check_throws(ctx, &expected, &closure)? {
            ThrowsOutcome::Passed(_) => Ok(SwiftValue::Void),
            ThrowsOutcome::Failed(detail) => {
                session::record_issue(format!("Expectation failed: {detail}"), node.line());
                Ok(SwiftValue::Void)
            }
        };
    }
    let Some(operand) = node.first_child() else {
        session::record_issue("Expectation failed: empty #expect()".into(), node.line());
        return Ok(SwiftValue::Void);
    };
    match evaluate(ctx, &operand)? {
        Outcome::Passed => Ok(SwiftValue::Void),
        Outcome::Failed(detail) => {
            session::record_issue(format!("Expectation failed: {detail}"), operand.line());
            Ok(SwiftValue::Void)
        }
    }
}

/// `#require(expr)` — hard check. On failure records an issue and aborts the
/// test body via a throw signal; on success unwraps an optional (a present
/// optional is already its wrapped value) and yields it. Use outside a test is
/// a hard error.
pub fn require_macro(ctx: &mut dyn StdContext, node: &Node<'static>) -> StdResult {
    if !session::is_active() {
        return Err(trap("#require used outside a test"));
    }
    if let Some((expected, closure)) = throws_operands(node) {
        return match check_throws(ctx, &expected, &closure)? {
            // Per Apple, a satisfied `try #require(throws:)` returns the thrown
            // error so the caller can inspect it (`let e = try #require(…)`).
            ThrowsOutcome::Passed(thrown) => Ok(thrown.unwrap_or(SwiftValue::Void)),
            ThrowsOutcome::Failed(detail) => {
                session::record_issue(
                    format!("Required expectation failed: {detail}"),
                    node.line(),
                );
                session::mark_aborted();
                Err(StdError::Throw(SwiftValue::Void))
            }
        };
    }
    let Some(operand) = node.first_child() else {
        return Ok(SwiftValue::Void);
    };
    let value = ctx.eval_node(&operand)?;
    let satisfied = match &value {
        SwiftValue::Nil => false,
        SwiftValue::Bool(b) => *b,
        _ => true,
    };
    if satisfied {
        return Ok(value);
    }
    session::record_issue(
        format!("Required expectation failed: {}", render::expr(&operand)),
        operand.line(),
    );
    session::mark_aborted();
    // Unwind the test body. The runner distinguishes this from a user throw by
    // the session's `aborted` flag, so the sentinel value is irrelevant.
    Err(StdError::Throw(SwiftValue::Void))
}

/// Check `operand`, evaluating each subexpression exactly once.
///
/// For a comparison (`==`, `<`, …) the two operands are evaluated once each and
/// the operator applied to the captured values, so an impure operand (a
/// counter, a logging call) runs a single time and the failure detail reuses
/// those same values — no re-evaluation on failure. Operands needing contextual
/// typing (a leading-dot `.case`) or a non-structural custom operator fall back
/// to one whole-operand evaluation, which preserves the interpreter's operator
/// resolution at the cost of a spelling-only detail.
fn evaluate(ctx: &mut dyn StdContext, operand: &Node<'static>) -> Result<Outcome, StdError> {
    if operand.kind() == NodeKind::BinaryExpr {
        let op = operand.text().unwrap_or_default();
        if is_comparison(&op) {
            let mut it = operand.children();
            if let (Some(lhs), Some(rhs)) = (it.next(), it.next()) {
                let l = ctx.eval_node(&lhs);
                let r = ctx.eval_node(&rhs);
                if let (Ok(l), Ok(r)) = (l, r) {
                    if let Some(passed) = compare(&op, &l, &r) {
                        return Ok(if passed {
                            Outcome::Passed
                        } else {
                            Outcome::Failed(binary_detail(operand, &lhs, &l, &rhs, &r))
                        });
                    }
                }
                // Contextual-typing operand or a custom operator we cannot apply
                // structurally: fall back to one whole-operand evaluation.
            }
        }
    }
    whole_operand(ctx, operand)
}

/// Evaluate `operand` once as a whole and require it to be a `Bool`.
fn whole_operand(ctx: &mut dyn StdContext, operand: &Node<'static>) -> Result<Outcome, StdError> {
    let value = ctx.eval_node(operand)?;
    match value.as_bool() {
        Some(true) => Ok(Outcome::Passed),
        Some(false) => Ok(Outcome::Failed(bool_detail(operand))),
        None => Err(trap("#expect requires a Bool expression")),
    }
}

/// Apply a comparison operator to two already-evaluated values, mirroring the
/// interpreter's structural equality for nil/reference/enum/struct operands and
/// otherwise deferring to the shared scalar operator table. Returns `None` when
/// the values cannot be compared structurally (e.g. a custom `Comparable`).
fn compare(op: &str, l: &SwiftValue, r: &SwiftValue) -> Option<bool> {
    if (op == "==" || op == "!=")
        && matches!(
            (l, r),
            (SwiftValue::Nil, _)
                | (_, SwiftValue::Nil)
                | (SwiftValue::Object(_), _)
                | (_, SwiftValue::Object(_))
                | (SwiftValue::Enum(_), _)
                | (_, SwiftValue::Enum(_))
                | (SwiftValue::Struct(_), _)
                | (_, SwiftValue::Struct(_))
        )
    {
        let same = l == r;
        return Some(if op == "==" { same } else { !same });
    }
    ops::binary(op, l, r).ok().and_then(|v| v.as_bool())
}

fn is_comparison(op: &str) -> bool {
    matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=")
}

/// Build the `#expect` failure detail for a comparison from captured operand
/// values, appending each non-literal operand's value (plan §3.5).
fn binary_detail(
    operand: &Node<'static>,
    lhs: &Node<'static>,
    l: &SwiftValue,
    rhs: &Node<'static>,
    r: &SwiftValue,
) -> String {
    let mut detail = format!("{} → false", render::expr(operand));
    if !is_literal(lhs.kind()) {
        detail.push_str(&format!("\n  {} → {l}", render::expr(lhs)));
    }
    if !is_literal(rhs.kind()) {
        detail.push_str(&format!("\n  {} → {r}", render::expr(rhs)));
    }
    detail
}

/// Failure detail for a non-comparison operand: a bare boolean identifier or
/// member gets `→ false`; anything else shows just its spelling.
fn bool_detail(operand: &Node<'static>) -> String {
    let spelling = render::expr(operand);
    match operand.kind() {
        NodeKind::IdentExpr | NodeKind::MemberExpr => format!("{spelling} → false"),
        _ => spelling,
    }
}

/// `Issue.record(_: String)` — record a manual soft failure. Like a failing
/// `#expect`, it records against the current session and returns normally so
/// the test body continues. No source location is available at this static
/// call, so the issue is recorded at line 0; the runner (`run_one`) detects
/// that sentinel and attributes it to the test's own declaration line instead
/// of showing a bogus `<unknown>` location.
pub fn issue_record(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    if !session::is_active() {
        return Err(trap("Issue.record used outside a test"));
    }
    let message = args
        .first()
        .map(|arg| ctx.display(&arg.value))
        .unwrap_or_default();
    session::record_issue(format!("Issue recorded: {message}"), 0);
    Ok(SwiftValue::Void)
}

/// The result of a `#expect(throws:)` / `#require(throws:)` check, carrying the
/// thrown error (for `#require`'s return value) or a preformatted failure.
enum ThrowsOutcome {
    Passed(Option<SwiftValue>),
    Failed(String),
}

/// The `throws:`-labelled subject and trailing closure of a throws-matcher
/// `#expect(throws: …) { … }`, or `None` for the ordinary boolean form.
fn throws_operands(node: &Node<'static>) -> Option<(Node<'static>, Node<'static>)> {
    let mut expected = None;
    let mut closure = None;
    for child in node.children() {
        if child.arg_label().as_deref() == Some("throws") {
            expected = Some(child);
        } else if child.kind() == NodeKind::ClosureExpr {
            closure = Some(child);
        }
    }
    Some((expected?, closure?))
}

/// What a `throws:` subject expects: an error type by name (`E.self`,
/// `Never.self`) or a specific error instance to equal (`MyError.bad`).
enum Expected {
    Type(String),
    Instance(SwiftValue),
}

/// The type name of a syntactic `T.self` metatype subject, read from the AST so
/// `Never.self` (an undeclared type in the runtime) resolves without an
/// evaluation that would fail with "unknown variable: Never".
fn metatype_name(node: &Node<'static>) -> Option<String> {
    if node.kind() == NodeKind::MemberExpr && node.text().as_deref() == Some("self") {
        node.first_child().map(|base| render::expr(&base))
    } else {
        None
    }
}

/// Run `closure` and match its outcome against `expected` — a type metatype
/// (`E.self`), the special `Never.self` (must not throw), or an error instance
/// (equality). A trap inside the closure is not a throw and propagates.
fn check_throws(
    ctx: &mut dyn StdContext,
    expected_node: &Node<'static>,
    closure_node: &Node<'static>,
) -> Result<ThrowsOutcome, StdError> {
    let expected = match metatype_name(expected_node) {
        Some(name) => Expected::Type(name),
        None => match ctx.eval_node(expected_node)? {
            SwiftValue::Metatype(name) => Expected::Type(name),
            other => Expected::Instance(other),
        },
    };
    let closure = ctx.eval_node(closure_node)?;
    let id = match closure {
        SwiftValue::Closure(id) | SwiftValue::Function(id) => id,
        _ => return Err(trap("#expect(throws:) requires a closure")),
    };
    let result = ctx.call_closure(id, Vec::new());
    match expected {
        Expected::Type(name) if name == "Never" => match result {
            Ok(_) => Ok(ThrowsOutcome::Passed(None)),
            Err(StdError::Throw(thrown)) => Ok(ThrowsOutcome::Failed(format!(
                "expected no error to be thrown, but {} was thrown",
                ctx.display(&thrown)
            ))),
            Err(other) => Err(other),
        },
        Expected::Type(name) => match result {
            Ok(_) => Ok(ThrowsOutcome::Failed(format!(
                "expected error of type {name} to be thrown, but no error was thrown"
            ))),
            Err(StdError::Throw(thrown)) => {
                let actual = thrown.type_name();
                if actual == name {
                    Ok(ThrowsOutcome::Passed(Some(thrown)))
                } else {
                    Ok(ThrowsOutcome::Failed(format!(
                        "expected error of type {name} to be thrown, but {actual} was thrown"
                    )))
                }
            }
            Err(other) => Err(other),
        },
        Expected::Instance(expected) => match result {
            Ok(_) => Ok(ThrowsOutcome::Failed(format!(
                "expected {} to be thrown, but no error was thrown",
                ctx.display(&expected)
            ))),
            Err(StdError::Throw(thrown)) => {
                if thrown == expected {
                    Ok(ThrowsOutcome::Passed(Some(thrown)))
                } else {
                    Ok(ThrowsOutcome::Failed(format!(
                        "expected {} to be thrown, but {} was thrown",
                        ctx.display(&expected),
                        ctx.display(&thrown)
                    )))
                }
            }
            Err(other) => Err(other),
        },
    }
}

fn trap(message: &str) -> StdError {
    StdError::Error(EvalError::Trap(message.to_string()))
}

fn is_literal(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::IntegerLiteral
            | NodeKind::FloatLiteral
            | NodeKind::BoolLiteral
            | NodeKind::NilLiteral
            | NodeKind::StringLiteral
    )
}
