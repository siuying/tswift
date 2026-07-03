//! Static-type recovery for expressions.
//!
//! The runtime flattens optionals (absent = `Nil`, present = the wrapped value),
//! so by the time a value reaches `print` or method dispatch the "this was an
//! optional" fact is gone. [`Interpreter::static_type_of`] recovers it from
//! *written* type information — binding annotations, declared return types, and
//! cast targets — degrading gracefully to `None` (today's behavior) whenever the
//! static type is unrecoverable.

use tswift_frontend::{Node, NodeKind};

use super::Interpreter;
use crate::value::SwiftValue;

impl<'w> Interpreter<'w> {
    /// The statically-written type of `expr`, as annotation text (`Int?`,
    /// `[String?]`, …), or `None` when it cannot be recovered.
    ///
    /// This is type-level metadata only: it is never used for coercion, and a
    /// `None` result means "fall back to the value-directed behavior".
    ///
    /// Wired into the describe and dispatch seams by the stages that build on
    /// this foundation; exercised by unit tests until then.
    #[allow(dead_code)]
    pub(super) fn static_type_of(&self, expr: &Node<'static>) -> Option<String> {
        match expr.kind() {
            // Identifier → the referenced binding's written annotation.
            NodeKind::IdentExpr => {
                let name = expr.text()?;
                self.env.declared_type_of(&name).map(|t| t.to_string())
            }
            // Call to a user function → its declared return type. Method calls
            // and builtins are not resolved here (graceful `None`).
            NodeKind::CallExpr => {
                let callee = expr.children().next()?;
                if callee.kind() != NodeKind::IdentExpr {
                    return None;
                }
                let name = callee.text()?;
                match self.env.get(&name) {
                    Some(SwiftValue::Function(id)) => self.funcs[id].return_type.clone(),
                    _ => None,
                }
            }
            // `expr as T` / `expr as? T` / `expr as! T` → the cast target. `as?`
            // yields an optional, so mark it as such. `is` yields Bool and is
            // not a useful static type here.
            NodeKind::CastExpr => {
                let op = expr.op_text().unwrap_or_default();
                if op == "is" {
                    return None;
                }
                let target = expr.children().nth(1).and_then(|t| t.text())?;
                // `as?` yields an optional. If the target is already optional
                // (`x as? Int?`) we leave it as-is rather than nesting to
                // `Int??` — our consumers only ever ask `is_optional()`, for
                // which single vs. double optional is indistinguishable, so the
                // flattening is harmless.
                if expr.is_optional_cast() && !target.trim_end().ends_with('?') {
                    Some(format!("{target}?"))
                } else {
                    Some(target)
                }
            }
            _ => None,
        }
    }
}
