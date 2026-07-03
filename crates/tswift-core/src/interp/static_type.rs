//! Static-type recovery for expressions.
//!
//! The runtime flattens optionals (absent = `Nil`, present = the wrapped value),
//! so by the time a value reaches `print` or method dispatch the "this was an
//! optional" fact is gone. [`Interpreter::static_type_of`] recovers it from
//! *written* type information — binding annotations, declared return types, and
//! cast targets — degrading gracefully to `None` (today's behavior) whenever the
//! static type is unrecoverable.

use tswift_frontend::{Node, NodeKind, TypeRepr};

use super::Interpreter;
use crate::value::SwiftValue;

impl<'w> Interpreter<'w> {
    /// The statically-written type of `expr`, as annotation text (`Int?`,
    /// `[String?]`, …), or `None` when it cannot be recovered.
    ///
    /// This is type-level metadata only: it is never used for coercion, and a
    /// `None` result means "fall back to the value-directed behavior".
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
            // Array/dictionary literal → a type synthesized from the elements'
            // shape (Stage 2a). Only the optionality bit is meaningful; the
            // element base is a placeholder `T` and must never be used for
            // coercion.
            NodeKind::ArrayLiteral | NodeKind::DictLiteral => self.synthesize_literal_type(expr),
            _ => None,
        }
    }

    /// Best-effort static type for an array/dictionary literal, capturing only
    /// whether elements are optional (`[T?]`, `[T: T?]`, nested `[[T?]]`). An
    /// element is optional when it is a `nil` literal, an `Optional(x)`
    /// construction, an identifier whose declared type is optional, an `as?`
    /// cast, or a nested literal that is itself optional-bearing.
    ///
    /// Returns `None` when no optionality is detected — there is nothing to
    /// recover, so the describe path keeps its current behavior.
    fn synthesize_literal_type(&self, node: &Node<'static>) -> Option<String> {
        match node.kind() {
            NodeKind::ArrayLiteral => {
                let elem = self.synthesize_element_type(node.children())?;
                Some(format!("[{elem}]"))
            }
            NodeKind::DictLiteral => {
                // Dict children are a flat key, value, key, value, … sequence;
                // synthesize from the value positions (odd indices). Keys are
                // assumed non-optional (`T`).
                let values = node
                    .children()
                    .enumerate()
                    .filter(|(i, _)| i % 2 == 1)
                    .map(|(_, c)| c);
                let val = self.synthesize_element_type(values)?;
                Some(format!("[T: {val}]"))
            }
            _ => None,
        }
    }

    /// The synthesized element type for a homogeneous run of literal element
    /// expressions: `T?` when any element is optional, a nested literal's
    /// synthesized type when elements are themselves collections, or `None`
    /// when no optionality is present anywhere.
    fn synthesize_element_type(
        &self,
        elems: impl Iterator<Item = Node<'static>>,
    ) -> Option<String> {
        let mut optional = false;
        let mut nested: Option<String> = None;
        for e in elems {
            match e.kind() {
                NodeKind::ArrayLiteral | NodeKind::DictLiteral => {
                    if nested.is_none() {
                        nested = self.synthesize_literal_type(&e);
                    }
                }
                _ => {
                    if self.element_is_optional(&e) {
                        optional = true;
                    }
                }
            }
        }
        match nested {
            Some(inner) => Some(if optional { format!("{inner}?") } else { inner }),
            None if optional => Some("T?".to_string()),
            None => None,
        }
    }

    /// Whether a single literal-element expression is statically optional.
    fn element_is_optional(&self, e: &Node<'static>) -> bool {
        match e.kind() {
            NodeKind::NilLiteral => true,
            // `Optional(x)` construction.
            NodeKind::CallExpr => {
                e.children()
                    .next()
                    .filter(|c| c.kind() == NodeKind::IdentExpr)
                    .and_then(|c| c.text())
                    .as_deref()
                    == Some("Optional")
            }
            NodeKind::IdentExpr => e
                .text()
                .and_then(|name| self.env.declared_type_of(&name))
                .is_some_and(|t| TypeRepr::parse(&t).is_optional()),
            NodeKind::CastExpr => self
                .static_type_of(e)
                .is_some_and(|t| TypeRepr::parse(&t).is_optional()),
            _ => false,
        }
    }
}
