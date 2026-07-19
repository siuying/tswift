//! The `#expect` / `#require` freestanding-macro builtins.
//!
//! Registered on the interpreter via `register_macro` (same seam as
//! `#Predicate`), so they are not expanded to Swift AST — each handler
//! inspects the `CompilerDirective` node's operand, evaluates it, and records
//! against the current [`crate::session`] (plan §3).

use tswift_core::{EvalError, StdContext, StdError, StdResult, SwiftValue};
use tswift_frontend::{Node, NodeKind};

use crate::render;
use crate::session;

/// `#expect(expr)` — soft check. Records an issue on failure but lets the test
/// body continue.
pub fn expect_macro(ctx: &mut dyn StdContext, node: &Node<'static>) -> StdResult {
    let Some(operand) = node.first_child() else {
        return Ok(SwiftValue::Void);
    };
    let value = ctx.eval_node(&operand)?;
    if value.as_bool() == Some(true) {
        return Ok(SwiftValue::Void);
    }
    session::record_issue(
        format!("Expectation failed: {}", failure_detail(ctx, &operand)),
        operand.line(),
    );
    Ok(SwiftValue::Void)
}

/// `#require(expr)` — hard check. On failure records an issue and aborts the
/// test body via a throw signal; on success unwraps an optional (a present
/// optional is already its wrapped value) and yields it.
pub fn require_macro(ctx: &mut dyn StdContext, node: &Node<'static>) -> StdResult {
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

/// Build the `#expect` failure detail: the expression spelling, plus operand
/// values for a binary comparison or a bare boolean identifier (plan §3.5).
fn failure_detail(ctx: &mut dyn StdContext, operand: &Node<'static>) -> String {
    let spelling = render::expr(operand);
    match operand.kind() {
        NodeKind::BinaryExpr => {
            let mut it = operand.children();
            let (lhs, rhs) = (it.next(), it.next());
            let mut detail = format!("{spelling} → false");
            if let Some(lhs) = lhs {
                if let Some(line) = operand_value(ctx, &lhs) {
                    detail.push_str(&format!("\n  {} → {line}", render::expr(&lhs)));
                }
            }
            if let Some(rhs) = rhs {
                if let Some(line) = operand_value(ctx, &rhs) {
                    detail.push_str(&format!("\n  {} → {line}", render::expr(&rhs)));
                }
            }
            detail
        }
        NodeKind::IdentExpr | NodeKind::MemberExpr => format!("{spelling} → false"),
        _ => spelling,
    }
}

/// Evaluate an operand purely for its display value, suppressing side-effect
/// errors. Literals are skipped (their spelling already shows the value).
fn operand_value(ctx: &mut dyn StdContext, node: &Node<'static>) -> Option<String> {
    if is_literal(node.kind()) {
        return None;
    }
    match ctx.eval_node(node) {
        Ok(value) => Some(value.to_string()),
        Err(StdError::Throw(_)) | Err(StdError::Error(EvalError::Trap(_))) => None,
        Err(_) => None,
    }
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
