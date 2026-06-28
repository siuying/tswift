use tswift_frontend::{Node, NodeKind};

use super::{Eval, EvalError, Interpreter, Signal};
use crate::value::SwiftValue;

impl<'w> Interpreter<'w> {
    pub(super) fn eval_switch(&mut self, node: &Node<'static>) -> Eval {
        let kids: Vec<Node<'static>> = node.children().collect();
        let subject_node = kids
            .first()
            .ok_or_else(|| EvalError::Unsupported("switch without a subject".into()))?;
        let subject = self.eval(subject_node)?;
        let cases: Vec<Node<'static>> = kids[1..]
            .iter()
            .copied()
            .filter(|c| c.kind() == NodeKind::CaseClause)
            .collect();

        // Find the first matching case.
        let mut chosen = None;
        for (i, case) in cases.iter().enumerate() {
            if let Some(binds) = self.case_matches(case, &subject)? {
                chosen = Some((i, binds));
                break;
            }
        }

        let Some((start, mut binds)) = chosen else {
            return Ok(SwiftValue::Void);
        };
        let mut idx = start;
        loop {
            let (_, body) = case_parts(&cases[idx]);
            self.env.push();
            for (name, value) in &binds {
                self.env.declare(name, value.clone(), false);
            }
            let mut fell_through = false;
            let mut propagate = None;
            for stmt in &body {
                match self.eval(stmt) {
                    Ok(_) => {}
                    Err(Signal::Fallthrough) => {
                        fell_through = true;
                        break;
                    }
                    Err(Signal::Break(None)) => break,
                    Err(other) => {
                        propagate = Some(other);
                        break;
                    }
                }
            }
            self.env.pop();
            if let Some(sig) = propagate {
                return Err(sig);
            }
            if fell_through && idx + 1 < cases.len() {
                idx += 1;
                binds = Vec::new();
                continue;
            }
            break;
        }
        Ok(SwiftValue::Void)
    }

    /// Whether `case` matches `subject`, returning the names it binds.
    fn case_matches(
        &mut self,
        case: &Node<'static>,
        subject: &SwiftValue,
    ) -> Result<Option<Vec<(String, SwiftValue)>>, Signal> {
        let info = case.case_info();
        if info.is_default {
            return Ok(Some(Vec::new()));
        }
        let (patterns, _) = case_parts(case);
        for pattern in patterns {
            if let Some(binds) = self.match_pattern(&pattern, subject)? {
                if let Some(guard) = info.where_expr {
                    self.env.push();
                    for (name, value) in &binds {
                        self.env.declare(name, value.clone(), false);
                    }
                    let pass = self.eval_condition(&guard);
                    self.env.pop();
                    if !pass? {
                        continue;
                    }
                }
                return Ok(Some(binds));
            }
        }
        Ok(None)
    }

    /// Try to match a single pattern against `subject`. `Ok(Some(binds))` on a
    /// match (with any bound names), `Ok(None)` on a non-match.
    pub(super) fn match_pattern(
        &mut self,
        pattern: &Node<'static>,
        subject: &SwiftValue,
    ) -> Result<Option<Vec<(String, SwiftValue)>>, Signal> {
        match pattern.kind() {
            NodeKind::WildcardPattern => Ok(Some(Vec::new())),
            NodeKind::NamePattern => {
                let name = pattern.text().unwrap_or_default();
                Ok(Some(vec![(name, subject.clone())]))
            }
            NodeKind::RangePattern => {
                let bounds: Vec<Node<'static>> = pattern.children().collect();
                let marker = pattern.text();
                // One-sided range patterns carry a single bound tagged by
                // direction: `..<n` (upTo), `...n` (through), `n...` (from).
                if bounds.len() == 1 {
                    let bound = self.eval(&bounds[0])?;
                    let within = match (subject, &bound) {
                        (SwiftValue::Int(s), SwiftValue::Int(b)) => match marker.as_deref() {
                            Some("from") => s.raw >= b.raw,
                            Some("through") => s.raw <= b.raw,
                            Some("upTo") => s.raw < b.raw,
                            _ => return Ok(None),
                        },
                        _ => return Ok(None),
                    };
                    return Ok(if within { Some(Vec::new()) } else { None });
                }
                if bounds.len() != 2 {
                    return Ok(None);
                }
                let lo = self.eval(&bounds[0])?;
                let hi = self.eval(&bounds[1])?;
                let inclusive = marker.as_deref() == Some("...");
                if let (SwiftValue::Int(s), SwiftValue::Int(a), SwiftValue::Int(b)) =
                    (subject, &lo, &hi)
                {
                    let within = s.raw >= a.raw
                        && (if inclusive {
                            s.raw <= b.raw
                        } else {
                            s.raw < b.raw
                        });
                    return Ok(if within { Some(Vec::new()) } else { None });
                }
                Ok(None)
            }
            NodeKind::EnumCasePattern => {
                let case_name = pattern.op_text().unwrap_or_default();
                // The leading `TypeIdent` (e.g. the `E` in `E.bad`) is not a
                // sub-pattern; only payload bindings are.
                let subs: Vec<Node<'static>> = pattern
                    .children()
                    .filter(|c| c.kind() != NodeKind::TypeRef)
                    .collect();
                // Optional patterns desugar to `.some`/`.none`.
                if case_name == "some" {
                    if matches!(subject, SwiftValue::Nil) {
                        return Ok(None);
                    }
                    return match subs.first() {
                        Some(p) => self.match_pattern(p, subject),
                        None => Ok(Some(Vec::new())),
                    };
                }
                if case_name == "none" {
                    return Ok(if matches!(subject, SwiftValue::Nil) {
                        Some(Vec::new())
                    } else {
                        None
                    });
                }
                let SwiftValue::Enum(e) = subject else {
                    return Ok(None);
                };
                if e.case != case_name {
                    return Ok(None);
                }
                if !subs.is_empty() && subs.len() != e.payload.len() {
                    return Ok(None);
                }
                let mut all = Vec::new();
                for (sub, item) in subs.iter().zip(e.payload.iter()) {
                    match self.match_pattern(sub, item)? {
                        Some(b) => all.extend(b),
                        None => return Ok(None),
                    }
                }
                Ok(Some(all))
            }
            // `<pattern> as Type` - a cast pattern matches only when the
            // subject's dynamic type is `Type`, then binds the inner pattern.
            NodeKind::CastExpr => {
                let kids: Vec<Node<'static>> = pattern.children().collect();
                let Some(inner) = kids.first() else {
                    return Ok(None);
                };
                let ty = kids.get(1).and_then(|t| t.text()).unwrap_or_default();
                if self.value_is_type(subject, &ty) {
                    self.match_pattern(inner, subject)
                } else {
                    Ok(None)
                }
            }
            NodeKind::TuplePattern => {
                let SwiftValue::Tuple(items, _) = subject else {
                    return Ok(None);
                };
                let subs: Vec<Node<'static>> = pattern.children().collect();
                if subs.len() != items.len() {
                    return Ok(None);
                }
                let mut all = Vec::new();
                for (sub, item) in subs.iter().zip(items.iter()) {
                    match self.match_pattern(sub, item)? {
                        Some(b) => all.extend(b),
                        None => return Ok(None),
                    }
                }
                Ok(Some(all))
            }
            // An expression pattern: match by equality.
            _ => {
                let v = self.eval(pattern)?;
                Ok(if values_equal(&v, subject) {
                    Some(Vec::new())
                } else {
                    None
                })
            }
        }
    }
}

/// Split a `case` clause into (patterns, body-statements). Patterns are the
/// leading non-statement children; the body is everything from the first
/// statement onward.
fn case_parts(case: &Node<'static>) -> (Vec<Node<'static>>, Vec<Node<'static>>) {
    let mut patterns = Vec::new();
    let mut body = Vec::new();
    let mut in_body = false;
    for child in case.children() {
        // The `where` guard is read separately via `case_info()`, not matched
        // as a pattern.
        if child.kind() == NodeKind::WhereClause {
            continue;
        }
        if !in_body && is_statement_kind(child.kind()) {
            in_body = true;
        }
        if in_body {
            body.push(child);
        } else {
            patterns.push(child);
        }
    }
    (patterns, body)
}

/// Whether a node kind is a statement (as opposed to a `switch` pattern).
fn is_statement_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::ExprStmt
            | NodeKind::Block
            | NodeKind::ReturnStmt
            | NodeKind::IfStmt
            | NodeKind::GuardStmt
            | NodeKind::ForStmt
            | NodeKind::WhileStmt
            | NodeKind::RepeatStmt
            | NodeKind::SwitchStmt
            | NodeKind::BreakStmt
            | NodeKind::ContinueStmt
            | NodeKind::FallthroughStmt
            | NodeKind::VarDecl
            | NodeKind::LetDecl
            | NodeKind::FuncDecl
    )
}

/// Structural value equality used by `switch` value patterns and `==`.
fn values_equal(a: &SwiftValue, b: &SwiftValue) -> bool {
    match (a, b) {
        (SwiftValue::Int(x), SwiftValue::Int(y)) => x.raw == y.raw,
        (SwiftValue::Double(x), SwiftValue::Double(y)) => x == y,
        (SwiftValue::Bool(x), SwiftValue::Bool(y)) => x == y,
        (SwiftValue::Str(x), SwiftValue::Str(y)) => x == y,
        (SwiftValue::Nil, SwiftValue::Nil) => true,
        (SwiftValue::Tuple(x, _), SwiftValue::Tuple(y, _)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| values_equal(p, q))
        }
        (SwiftValue::Enum(x), SwiftValue::Enum(y)) => {
            x.type_name == y.type_name
                && x.case == y.case
                && x.payload.len() == y.payload.len()
                && x.payload
                    .iter()
                    .zip(&y.payload)
                    .all(|(p, q)| values_equal(p, q))
        }
        _ => false,
    }
}
