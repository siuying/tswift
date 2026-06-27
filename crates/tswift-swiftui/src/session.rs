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

use crate::{child_id, ACTION_FIELD, BINDING_FIELD, CHILDREN_FIELD};

/// Coerce a host `set` payload to the binding's current value type, so each
/// control writes a well-typed value (and a `Toggle<Bool>` can't be corrupted
/// by a stray string). Returns `None` for a missing or incompatible payload,
/// including a number that doesn't fit the binding's integer width.
fn coerce_binding_value(current: &SwiftValue, incoming: Option<&SwiftValue>) -> Option<SwiftValue> {
    let incoming = incoming?;
    match (current, incoming) {
        (SwiftValue::Bool(_), SwiftValue::Bool(b)) => Some(SwiftValue::Bool(*b)),
        (SwiftValue::Str(_), SwiftValue::Str(s)) => Some(SwiftValue::Str(s.clone())),
        // Integer bindings keep their declared width/sign: the incoming number
        // must be integral and in range, else the write is rejected (no silent
        // truncation or saturation that would corrupt `@State`).
        (SwiftValue::Int(cur), SwiftValue::Int(i)) => fit_int(i.raw, cur.width),
        (SwiftValue::Int(cur), SwiftValue::Double(d)) => {
            (d.is_finite() && d.fract() == 0.0).then(|| fit_int(*d as i128, cur.width))?
        }
        // Double bindings (a `Slider`) accept any finite number.
        (SwiftValue::Double(_), SwiftValue::Double(d)) if d.is_finite() => {
            Some(SwiftValue::Double(*d))
        }
        (SwiftValue::Double(_), SwiftValue::Int(i)) => Some(SwiftValue::Double(i.raw as f64)),
        _ => None,
    }
}

/// Build an integer of the binding's `width` from `raw`, or `None` if it is out
/// of range — so a control can never push an out-of-bounds value into a
/// fixed-width / unsigned `@State`.
fn fit_int(raw: i128, width: tswift_core::IntWidth) -> Option<SwiftValue> {
    let v = tswift_core::IntValue::new(raw, width);
    v.in_range().then_some(SwiftValue::Int(v))
}

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
        match event.event.as_str() {
            "tap" => {
                if let Some(closure_id) = find_action(&tree, &event.id) {
                    self.interp.invoke_closure(closure_id, Vec::new())?;
                }
            }
            // A control's new value (a `Toggle` bool, a `TextField` string, a
            // `Slider`/`Stepper` number) written through its `Binding`, so the
            // bound `@State` updates before re-render. The payload is coerced to
            // the binding's current value type; an incompatible or missing
            // payload is ignored rather than corrupting the binding.
            "set" => {
                if let Some(binding) = find_binding(&tree, &event.id) {
                    let current = self.interp.get_member(&binding, "wrappedValue")?;
                    if let Some(new) = coerce_binding_value(&current, event.value.as_ref()) {
                        self.interp.set_member(&binding, "wrappedValue", new)?;
                    }
                }
            }
            _ => {}
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
                if let Some(found) = walk(child, &child_id(id, i, child), target) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(tree, "0", target)
}

/// Find the `Binding` value stashed on the control node at structural path
/// `target` (the `_binding` field a `Toggle`/input writes through).
pub fn find_binding(tree: &SwiftValue, target: &str) -> Option<SwiftValue> {
    fn walk(node: &SwiftValue, id: &str, target: &str) -> Option<SwiftValue> {
        let SwiftValue::Struct(obj) = node else {
            return None;
        };
        if id == target {
            return obj.get(BINDING_FIELD).cloned();
        }
        let prefix = format!("{id}.");
        if !target.starts_with(&prefix) {
            return None;
        }
        if let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) {
            for (i, child) in children.iter().enumerate() {
                if let Some(found) = walk(child, &child_id(id, i, child), target) {
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

    fn greeting_interp() -> Interpreter<'static> {
        let src = format!(
            "{PRELUDE}\n{}",
            r#"
struct GreetingView: View {
    @State private var formal = true
    var body: some View {
        VStack {
            Toggle("Formal", isOn: $formal)
            Text(formal ? "Good evening." : "Hey!")
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
    fn toggle_set_writes_through_binding_and_rerenders() {
        let mut interp = greeting_interp();
        let mut session = Session::new(&mut interp, "GreetingView").expect("session");

        let first = session.render().expect("render");
        let json = uiir::to_json(&first);
        assert!(json.contains(r#""isOn":true"#), "{json}");
        assert!(json.contains("Good evening."), "{json}");

        // Flip the Toggle (id "0.0") off via a `set` event carrying `false`.
        let off = Event {
            id: "0.0".into(),
            event: "set".into(),
            value: Some(SwiftValue::Bool(false)),
        };
        let after = session.dispatch(&off).expect("dispatch");
        let json = uiir::to_json(&after);
        assert!(json.contains(r#""isOn":false"#), "{json}");
        assert!(
            json.contains("Hey!"),
            "toggling should switch the greeting: {json}"
        );
    }

    fn textfield_interp() -> Interpreter<'static> {
        let src = format!(
            "{PRELUDE}\n{}",
            r#"
struct FormView: View {
    @State private var name = "World"
    var body: some View {
        VStack {
            TextField("Name", text: $name)
            Text("Hello, \(name)!")
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
    fn textfield_set_writes_string_through_binding() {
        let mut interp = textfield_interp();
        let mut session = Session::new(&mut interp, "FormView").expect("session");
        session.render().expect("render");

        let set = Event {
            id: "0.0".into(),
            event: "set".into(),
            value: Some(SwiftValue::Str("Swift".into())),
        };
        let after = session.dispatch(&set).expect("dispatch");
        let json = uiir::to_json(&after);
        assert!(json.contains(r#""text":"Swift""#), "field updates: {json}");
        assert!(
            json.contains("Hello, Swift!"),
            "binding drives the greeting: {json}"
        );
    }

    #[test]
    fn slider_set_writes_double_through_binding() {
        let src = format!(
            "{PRELUDE}\n{}",
            r#"
struct DimmerView: View {
    @State private var level = 0.25
    var body: some View {
        VStack {
            Slider(value: $level, in: 0...1)
            Text("\(level)")
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
        let mut session = Session::new(&mut interp, "DimmerView").expect("session");
        session.render().expect("render");

        let set = Event {
            id: "0.0".into(),
            event: "set".into(),
            value: Some(SwiftValue::Double(0.75)),
        };
        let after = session.dispatch(&set).expect("dispatch");
        let json = uiir::to_json(&after);
        assert!(json.contains(r#""value":0.75"#), "slider value: {json}");
        assert!(json.contains("0.75"), "binding drives the text: {json}");
    }

    #[test]
    fn dispatch_routes_to_a_keyed_foreach_row_control() {
        // A control nested inside a keyed `ForEach` row must be reachable by its
        // keyed id (`0.0.row.0`), exercising the child_id walker fix.
        let src = format!(
            "{PRELUDE}\n{}",
            r#"
struct RowsView: View {
    @State private var name = "World"
    var body: some View {
        VStack {
            ForEach(["row"], id: \.self) { _ in
                TextField("Name", text: $name)
            }
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
        let mut session = Session::new(&mut interp, "RowsView").expect("session");
        let first = session.render().expect("render");
        // VStack(0) > ForEach(0.0) > keyed TextField row (key "row").
        assert!(
            uiir::to_json(&first).contains(r#""id":"0.0.row""#),
            "keyed row id: {}",
            uiir::to_json(&first)
        );
        // A `set` to the keyed id must route through the binding.
        let set = Event {
            id: "0.0.row".into(),
            event: "set".into(),
            value: Some(SwiftValue::Str("Ada".into())),
        };
        let after = session.dispatch(&set).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains(r#""text":"Ada""#),
            "keyed-row event should reach the field: {}",
            uiir::to_json(&after)
        );
    }

    #[test]
    fn coerce_binding_value_enforces_types_and_int_ranges() {
        use tswift_core::{IntValue, IntWidth};
        // String binding accepts strings, rejects others.
        let s = SwiftValue::Str("a".into());
        assert_eq!(
            coerce_binding_value(&s, Some(&SwiftValue::Str("b".into()))),
            Some(SwiftValue::Str("b".into()))
        );
        assert_eq!(
            coerce_binding_value(&s, Some(&SwiftValue::Bool(true))),
            None
        );
        // Bool binding rejects a string.
        let b = SwiftValue::Bool(false);
        assert_eq!(
            coerce_binding_value(&b, Some(&SwiftValue::Str("x".into()))),
            None
        );
        // A UInt8 binding rejects an out-of-range or negative value.
        let u8b = SwiftValue::Int(IntValue::new(10, IntWidth::U8));
        assert_eq!(
            coerce_binding_value(&u8b, Some(&SwiftValue::int(300))),
            None
        );
        assert_eq!(coerce_binding_value(&u8b, Some(&SwiftValue::int(-1))), None);
        assert_eq!(
            coerce_binding_value(&u8b, Some(&SwiftValue::int(200))),
            Some(SwiftValue::Int(IntValue::new(200, IntWidth::U8)))
        );
        // Double binding accepts an int (widened) but rejects NaN.
        let d = SwiftValue::Double(1.0);
        assert_eq!(
            coerce_binding_value(&d, Some(&SwiftValue::int(3))),
            Some(SwiftValue::Double(3.0))
        );
        assert_eq!(
            coerce_binding_value(&d, Some(&SwiftValue::Double(f64::NAN))),
            None
        );
        // An integer binding rejects a non-integral double.
        let i = SwiftValue::int(0);
        assert_eq!(
            coerce_binding_value(&i, Some(&SwiftValue::Double(2.5))),
            None
        );
        assert_eq!(
            coerce_binding_value(&i, Some(&SwiftValue::Double(2.0))),
            Some(SwiftValue::int(2))
        );
    }

    #[test]
    fn malformed_set_event_is_ignored() {
        let mut interp = greeting_interp();
        let mut session = Session::new(&mut interp, "GreetingView").expect("session");
        session.render().expect("render");

        // A `set` carrying a non-bool payload must not corrupt the Bool binding.
        let bad = Event {
            id: "0.0".into(),
            event: "set".into(),
            value: Some(SwiftValue::Str("nope".into())),
        };
        let after = session.dispatch(&bad).expect("dispatch stays Ok");
        let json = uiir::to_json(&after);
        assert!(json.contains(r#""isOn":true"#), "state unchanged: {json}");
        assert!(json.contains("Good evening."), "{json}");
    }
}
