//! The render/dispatch loop — the stateful side of the host (plan §3.3/§3.4).
//!
//! A session instantiates the root `View` **once** so its `@State` (backed by a
//! shared reference box in the prelude) survives across renders, then:
//!   * `render` evaluates `body` into a UIIR view-value tree, and
//!   * `dispatch` routes an event to a node's captured action closure, runs it
//!     (mutating `@State`), and re-renders.
//!
//! Node identity is the structural path (`"0"`, `"0.1"`, …) shared with
//! `uiir`, so an event `id` from a host maps back to the same node.

use tswift_core::{EvalError, Interpreter, SwiftValue};

use crate::{ACTION_FIELD, CHILDREN_FIELD};

/// A discrete host→runtime event (plan §3.3): a node `id`, an event name, and
/// an optional payload value (e.g. a text field's new string).
#[derive(Debug, Clone)]
pub struct Event {
    pub id: String,
    pub event: String,
    pub value: Option<SwiftValue>,
}

/// A stateful render session over an interpreter that has already run the
/// program (so `root_type` and the prelude are declared). The root view
/// instance is created once and reused, preserving `@State`.
pub struct Session<'i, 'w> {
    interp: &'i mut Interpreter<'w>,
    instance: SwiftValue,
    /// The most recently rendered UIIR tree (for event routing).
    current: Option<SwiftValue>,
}

impl<'i, 'w> Session<'i, 'w> {
    /// Instantiate `root_type` once and start a session over it.
    pub fn new(interp: &'i mut Interpreter<'w>, root_type: &str) -> Result<Self, EvalError> {
        let instance = interp.make_struct(root_type, &[])?;
        Ok(Session {
            interp,
            instance,
            current: None,
        })
    }

    /// Evaluate the root view's `body` into a fresh UIIR tree, caching it for
    /// event routing.
    pub fn render(&mut self) -> Result<SwiftValue, EvalError> {
        let body = self.interp.get_member(&self.instance, "body")?;
        let tree = crate::resolve_root(self.interp, body).map_err(crate::std_error_to_eval)?;
        self.current = Some(tree.clone());
        Ok(tree)
    }

    /// The most recently rendered UIIR tree, if any (for diffing).
    pub fn current_tree(&self) -> Option<&SwiftValue> {
        self.current.as_ref()
    }

    /// Route `event` to the matching node's action and re-render. Unknown ids or
    /// nodes without an action are a no-op that still re-renders (the runtime
    /// stays the single source of truth).
    pub fn dispatch(&mut self, event: &Event) -> Result<SwiftValue, EvalError> {
        let tree = match &self.current {
            Some(tree) => tree.clone(),
            None => self.render()?,
        };
        if event.event == "tap" {
            if let Some(closure_id) = find_action(&tree, &event.id) {
                self.interp.invoke_closure(closure_id, Vec::new())?;
            }
        }
        self.render()
    }
}

/// Find the action closure id for the node at structural path `target` in
/// `tree`, matching the id scheme used by `uiir`.
pub fn find_action(tree: &SwiftValue, target: &str) -> Option<usize> {
    fn walk(node: &SwiftValue, id: &str, target: &str) -> Option<usize> {
        let SwiftValue::Struct(obj) = node else {
            return None;
        };
        if id == target {
            return match obj.get(ACTION_FIELD) {
                Some(SwiftValue::Closure(cid)) => Some(*cid),
                _ => None,
            };
        }
        // Only descend when `target` lies under this node's subtree.
        let prefix = format!("{id}.");
        if !target.starts_with(&prefix) {
            return None;
        }
        if let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) {
            for (i, child) in children.iter().enumerate() {
                if let Some(found) = walk(child, &format!("{id}.{i}"), target) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(tree, "0", target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{install, uiir, PRELUDE};

    fn counter_interp() -> Interpreter<'static> {
        let src = format!(
            "{PRELUDE}\n{}",
            r#"
struct CounterView: View {
    @State var count = 0
    var body: some View {
        VStack {
            Text("\(count)")
            Button("Increment") { count += 1 }
        }
    }
}
"#
        );
        let analysis = tswift_frontend::Analysis::analyze(&src, "t.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let out: &'static mut std::io::Sink = Box::leak(Box::new(std::io::sink()));
        let mut interp = Interpreter::new(out);
        install(&mut interp);
        interp.run(analysis).expect("run");
        interp
    }

    #[test]
    fn tap_mutates_state_and_rerenders() {
        let mut interp = counter_interp();
        let mut session = Session::new(&mut interp, "CounterView").expect("session");

        let first = session.render().expect("render");
        // The counter starts at 0; its first child is the Text node "0.0".
        assert!(uiir::to_json(&first).contains(r#""verbatim":"0""#));

        // The Button is the second child: structural id "0.1".
        let tap = Event {
            id: "0.1".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&tap).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains(r#""verbatim":"1""#),
            "tap should bump @State count to 1: {}",
            uiir::to_json(&after)
        );

        let after2 = session.dispatch(&tap).expect("dispatch");
        assert!(uiir::to_json(&after2).contains(r#""verbatim":"2""#));
    }
}
