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

use std::collections::HashMap;
use std::rc::Rc;

use tswift_core::{EvalError, Interpreter, SwiftValue};

use crate::{
    child_id, BINDING_FIELD, CHILDREN_FIELD, HANDLERS_FIELD, NAV_DESTINATION_FIELD, WATCH_FIELD,
};

/// Maximum `onChange` cascade passes per dispatch: a watcher's action may mutate
/// state a *second* watcher observes (chained state). We re-render and re-scan
/// until quiescent, bounded so a watcher toggling its own source cannot hang.
const MAX_WATCH_PASSES: usize = 64;

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
    /// Per-node `TabView` selection for tab views *without* a `selection:`
    /// binding (ADR-0013 §2), keyed by structural node id. A `select` event on
    /// such a node records its tag-or-index here; each render re-applies it into
    /// the node's `selection` arg (mirrors NavigationStack per-stack state).
    tab_selection: HashMap<String, SwiftValue>,
    /// Per-`NavigationStack` pushed-screen state (ADR-0013 §1), keyed by the
    /// stack's structural node id. Each entry is a captured `NavigationLink`
    /// destination (a `@ViewBuilder` closure re-evaluated fresh each render, or
    /// an eagerly-built destination view). A link tap appends; a `back` event
    /// pops. Each render appends the realized screens as children of the stack.
    nav_stack: HashMap<String, Vec<SwiftValue>>,
}

impl<'i, 'w> Session<'i, 'w> {
    /// Instantiate `root_type` once and start a session over it.
    pub fn new(interp: &'i mut Interpreter<'w>, root_type: &str) -> Result<Self, EvalError> {
        let instance = interp.make_struct(root_type, &[])?;
        Ok(Session {
            interp,
            instance,
            current: None,
            tab_selection: HashMap::new(),
            nav_stack: HashMap::new(),
        })
    }

    /// Evaluate the root view's `body` into a fresh UIIR tree, caching it for
    /// event routing.
    pub fn render(&mut self) -> Result<SwiftValue, EvalError> {
        let body = self.interp.get_member(&self.instance, "body")?;
        let tree = crate::resolve_root(self.interp, body).map_err(crate::std_error_to_eval)?;
        // Append each `NavigationStack`'s pushed screens (ADR-0013 §1) before the
        // tab-selection pass, so the tab pass sees the full (root + pushed) tree.
        let tree = self.apply_nav_stack(tree, "0")?;
        // Re-apply any per-node `TabView` selection owned by the session (the
        // no-binding case): the freshly evaluated `body` defaults each such tab
        // view to its first tab, so the session's stored selection overrides it.
        let tree = self.apply_tab_selection(tree, "0");
        self.current = Some(tree.clone());
        Ok(tree)
    }

    /// Override the `selection` arg of every `TabView` node lacking a
    /// `selection:` binding with the session's stored per-node selection (if
    /// any). Bound tab views read their selection from the binding, so they are
    /// left untouched. Walks with the same structural ids as `uiir`.
    fn apply_tab_selection(&self, node: SwiftValue, id: &str) -> SwiftValue {
        let SwiftValue::Struct(obj) = node else {
            return node;
        };
        let mut obj = (*obj).clone();
        if obj.type_name == "TabView" && !obj.fields.iter().any(|(k, _)| k == BINDING_FIELD) {
            if let Some(selected) = self.tab_selection.get(id) {
                if let Some(slot) = obj.fields.iter_mut().find(|(k, _)| k == "selection") {
                    slot.1 = selected.clone();
                }
            }
        }
        if let Some(pos) = obj.fields.iter().position(|(k, _)| k == CHILDREN_FIELD) {
            if let SwiftValue::Array(children) = &obj.fields[pos].1 {
                let kids: Vec<SwiftValue> = children.iter().cloned().collect();
                let mapped: Vec<SwiftValue> = kids
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let cid = child_id(id, i, &c);
                        self.apply_tab_selection(c, &cid)
                    })
                    .collect();
                obj.fields[pos].1 = SwiftValue::Array(Rc::new(mapped));
            }
        }
        SwiftValue::Struct(Rc::new(obj))
    }

    /// Append every `NavigationStack`'s pushed screens (ADR-0013 §1) as ordinary
    /// children, keyed by the stack's structural id. Each pushed destination is
    /// realized fresh — a `@ViewBuilder` closure is re-evaluated so the screen
    /// re-reads `@State`; an eager destination view is expanded as-is — and
    /// appended after the stack's root content (root first, topmost last). Walks
    /// with the same structural ids as `uiir`.
    fn apply_nav_stack(&mut self, node: SwiftValue, id: &str) -> Result<SwiftValue, EvalError> {
        let SwiftValue::Struct(obj) = node else {
            return Ok(node);
        };
        let mut obj = (*obj).clone();
        // Recurse into the existing children first (mapping to their ids).
        if let Some(pos) = obj.fields.iter().position(|(k, _)| k == CHILDREN_FIELD) {
            if let SwiftValue::Array(children) = &obj.fields[pos].1 {
                let kids: Vec<SwiftValue> = children.iter().cloned().collect();
                let mut mapped = Vec::with_capacity(kids.len());
                for (i, c) in kids.into_iter().enumerate() {
                    let cid = child_id(id, i, &c);
                    mapped.push(self.apply_nav_stack(c, &cid)?);
                }
                obj.fields[pos].1 = SwiftValue::Array(Rc::new(mapped));
            }
        }
        // Then, if this is a stack with pushed screens, append them as children.
        if obj.type_name == "NavigationStack" {
            if let Some(pushed) = self.nav_stack.get(id).cloned() {
                if !pushed.is_empty() {
                    let pos = match obj.fields.iter().position(|(k, _)| k == CHILDREN_FIELD) {
                        Some(pos) => pos,
                        None => {
                            obj.fields.push((
                                CHILDREN_FIELD.into(),
                                SwiftValue::Array(Rc::new(Vec::new())),
                            ));
                            obj.fields.len() - 1
                        }
                    };
                    let mut kids: Vec<SwiftValue> = match &obj.fields[pos].1 {
                        SwiftValue::Array(a) => a.iter().cloned().collect(),
                        _ => Vec::new(),
                    };
                    for dest in pushed {
                        if let Some(screen) = crate::realize_pushed_screen(self.interp, &dest)
                            .map_err(crate::std_error_to_eval)?
                        {
                            kids.push(screen);
                        }
                    }
                    obj.fields[pos].1 = SwiftValue::Array(Rc::new(kids));
                }
            }
        }
        Ok(SwiftValue::Struct(Rc::new(obj)))
    }

    /// The most recently rendered UIIR tree, if any (for diffing).
    pub fn current_tree(&self) -> Option<&SwiftValue> {
        self.current.as_ref()
    }

    /// Route `event` to the matching node and re-render. `set` writes a control's
    /// value through its `Binding`; every other event name (`tap`, `longPress`,
    /// `appear`, `disappear`, `submit`, …) routes into the node's `_handlers`
    /// map (ADR-0013 §3). Unknown ids or nodes without a matching handler are a
    /// no-op that still re-renders (the runtime stays the single source of
    /// truth). After the state mutation, `onChange(of:)` watchers are compared
    /// against the pre-event tree and fired before the caller diffs.
    pub fn dispatch(&mut self, event: &Event) -> Result<SwiftValue, EvalError> {
        let tree = match &self.current {
            Some(tree) => tree.clone(),
            None => self.render()?,
        };
        // Snapshot the watched-value baseline from the pre-event tree.
        let baseline = collect_watch_values(&tree);
        match event.event.as_str() {
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
            // A `TabView` tab selection (ADR-0013 §2): with a `selection:`
            // binding, write the new tag-or-index through it (the same binding
            // route `set` uses); without one, record it in the session's
            // per-node tab-selection state. Either way the next render reflects
            // it as the node's `selection` arg.
            "select" => {
                if let Some(binding) = find_binding(&tree, &event.id) {
                    let current = self.interp.get_member(&binding, "wrappedValue")?;
                    if let Some(new) = coerce_binding_value(&current, event.value.as_ref()) {
                        self.interp.set_member(&binding, "wrappedValue", new)?;
                    }
                } else if let Some(value) = &event.value {
                    self.tab_selection.insert(event.id.clone(), value.clone());
                }
            }
            // A `NavigationStack` back affordance (ADR-0013 §1): pop the topmost
            // pushed screen off the stack keyed by the event id. A `back` on a
            // stack with no pushed screens (only the root child) is a no-op.
            "back" => {
                if let Some(stack) = self.nav_stack.get_mut(&event.id) {
                    stack.pop();
                    if stack.is_empty() {
                        self.nav_stack.remove(&event.id);
                    }
                }
            }
            // Any other event routes to the node's handler map by name. A `tap`
            // on a `NavigationLink` is special-cased first (ADR-0013 §1): it
            // captures the link's destination onto the enclosing stack's pushed
            // state instead of running a handler.
            name => {
                if name == "tap" {
                    if let Some((stack_id, destination)) = find_nav_link(&tree, &event.id) {
                        self.nav_stack
                            .entry(stack_id)
                            .or_default()
                            .push(destination);
                    } else if let Some(closure_id) = find_handler(&tree, &event.id, name) {
                        self.interp.invoke_closure(closure_id, Vec::new())?;
                    }
                } else if let Some(closure_id) = find_handler(&tree, &event.id, name) {
                    self.interp.invoke_closure(closure_id, Vec::new())?;
                }
            }
        }
        let tree = self.render()?;
        self.run_watchers(baseline, tree)
    }

    /// Fire `onChange(of:)` watchers whose watched value differs from `baseline`
    /// (the pre-event snapshot), invoking each action with `(oldValue,
    /// newValue)`. A fired action may mutate state a *second* watcher observes,
    /// so we re-render and re-scan until quiescent (bounded by
    /// [`MAX_WATCH_PASSES`]). Returns the final rendered tree.
    fn run_watchers(
        &mut self,
        mut baseline: Vec<(String, usize, SwiftValue)>,
        mut tree: SwiftValue,
    ) -> Result<SwiftValue, EvalError> {
        for _ in 0..MAX_WATCH_PASSES {
            let current = collect_watchers(&tree);
            let mut fired = false;
            for (id, index, value, closure) in &current {
                if let Some((_, _, old)) = baseline
                    .iter()
                    .find(|(bid, bidx, _)| bid == id && bidx == index)
                {
                    if !values_equal(old, value) {
                        self.interp
                            .invoke_closure(*closure, vec![old.clone(), value.clone()])?;
                        fired = true;
                    }
                }
            }
            // Advance the baseline to the values just observed so an unchanged
            // watcher never re-fires and a fired one settles.
            baseline = current
                .into_iter()
                .map(|(id, index, value, _)| (id, index, value))
                .collect();
            if !fired {
                return Ok(tree);
            }
            tree = self.render()?;
        }
        Ok(tree)
    }
}

/// Compare two Swift values for `onChange` equality. Covers the scalar cases a
/// watched value realistically takes (`Int`/`Double`/`Bool`/`Str`/`Nil`); any
/// richer value falls back to its display string.
fn values_equal(a: &SwiftValue, b: &SwiftValue) -> bool {
    match (a, b) {
        (SwiftValue::Bool(x), SwiftValue::Bool(y)) => x == y,
        (SwiftValue::Int(x), SwiftValue::Int(y)) => x.raw == y.raw,
        (SwiftValue::Double(x), SwiftValue::Double(y)) => x == y,
        (SwiftValue::Str(x), SwiftValue::Str(y)) => x == y,
        (SwiftValue::Nil, SwiftValue::Nil) => true,
        _ => a.to_string() == b.to_string(),
    }
}

/// Collect every `onChange` watcher in `tree` as `(node id, watcher index,
/// value, action closure id)`, in structural order.
fn collect_watchers(tree: &SwiftValue) -> Vec<(String, usize, SwiftValue, usize)> {
    fn walk(node: &SwiftValue, id: &str, out: &mut Vec<(String, usize, SwiftValue, usize)>) {
        let SwiftValue::Struct(obj) = node else {
            return;
        };
        if let Some(SwiftValue::Array(watchers)) = obj.get(WATCH_FIELD) {
            for (i, w) in watchers.iter().enumerate() {
                if let SwiftValue::Struct(rec) = w {
                    let value = rec.get("value").cloned().unwrap_or(SwiftValue::Nil);
                    if let Some(SwiftValue::Closure(cid)) = rec.get("action") {
                        out.push((id.to_string(), i, value, *cid));
                    }
                }
            }
        }
        if let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) {
            for (i, child) in children.iter().enumerate() {
                walk(child, &child_id(id, i, child), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(tree, "0", &mut out);
    out
}

/// Collect just the `(id, index, value)` baseline of every watcher (the action
/// closure is irrelevant for the pre-event snapshot).
fn collect_watch_values(tree: &SwiftValue) -> Vec<(String, usize, SwiftValue)> {
    collect_watchers(tree)
        .into_iter()
        .map(|(id, index, value, _)| (id, index, value))
        .collect()
}

/// Find the handler closure id for `event` on the node at structural path
/// `target` in `tree`, matching the id scheme used by `uiir`. Looks up the
/// node's `_handlers` map (ADR-0013 §3): `tap` for a `Button` or
/// `.onTapGesture`, `longPress`/`appear`/`disappear`/`submit` for the
/// corresponding modifiers.
pub fn find_handler(tree: &SwiftValue, target: &str, event: &str) -> Option<usize> {
    fn walk(node: &SwiftValue, id: &str, target: &str, event: &str) -> Option<usize> {
        let SwiftValue::Struct(obj) = node else {
            return None;
        };
        if id == target {
            return match obj.get(HANDLERS_FIELD) {
                Some(SwiftValue::Struct(handlers)) => match handlers.get(event) {
                    Some(SwiftValue::Closure(cid)) => Some(*cid),
                    _ => None,
                },
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
                if let Some(found) = walk(child, &child_id(id, i, child), target, event) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(tree, "0", target, event)
}

/// Find the tapped `NavigationLink` at structural path `target` and its
/// enclosing `NavigationStack` (ADR-0013 §1). Returns `(stack id, captured
/// destination)` so the session can push the destination onto that stack. The
/// nearest ancestor `NavigationStack` id is threaded down the walk.
pub fn find_nav_link(tree: &SwiftValue, target: &str) -> Option<(String, SwiftValue)> {
    fn walk(
        node: &SwiftValue,
        id: &str,
        target: &str,
        stack_id: Option<&str>,
    ) -> Option<(String, SwiftValue)> {
        let SwiftValue::Struct(obj) = node else {
            return None;
        };
        let this_stack = if obj.type_name == "NavigationStack" {
            Some(id)
        } else {
            stack_id
        };
        if id == target {
            let dest = obj.get(NAV_DESTINATION_FIELD)?;
            return this_stack.map(|sid| (sid.to_string(), dest.clone()));
        }
        let prefix = format!("{id}.");
        if !target.starts_with(&prefix) {
            return None;
        }
        if let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) {
            for (i, child) in children.iter().enumerate() {
                if let Some(found) = walk(child, &child_id(id, i, child), target, this_stack) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(tree, "0", target, None)
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
    fn state_object_observes_published_mutations_across_renders() {
        // An `ObservableObject` (a class) owned by `@StateObject` is mutated
        // through its reference by a `Button` action; because the root view
        // instance is reused and the model is a reference, the next render
        // reflects it — no Combine publisher needed.
        let src = format!(
            "{PRELUDE}\n{}",
            r#"
class CounterModel: ObservableObject {
    @Published var count = 0
    func increment() { count += 1 }
}
struct CounterView: View {
    @StateObject var model = CounterModel()
    var body: some View {
        VStack {
            Text("Count: \(model.count)")
            Button("Increment") { model.increment() }
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
        let mut session = Session::new(&mut interp, "CounterView").expect("session");

        let first = session.render().expect("render");
        assert!(uiir::to_json(&first).contains("Count: 0"));

        // Two taps on the Increment button (id "0.1") drive the @Published count.
        let tap = Event {
            id: "0.1".into(),
            event: "tap".into(),
            value: None,
        };
        session.dispatch(&tap).expect("dispatch");
        let after = session.dispatch(&tap).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains("Count: 2"),
            "observed object should persist and reflect mutations: {}",
            uiir::to_json(&after)
        );
    }

    #[test]
    fn environment_object_is_injected_into_a_child_view() {
        // A root provides an `ObservableObject` via `.environmentObject(_)`; the
        // child reads it through `@EnvironmentObject` and a mutation through it
        // (no owner reference in scope) is reflected on the next render.
        let src = format!(
            "{PRELUDE}\n{}",
            r#"
class Settings: ObservableObject {
    @Published var theme = "dark"
    func toggle() { theme = theme == "dark" ? "light" : "dark" }
}
struct ChildView: View {
    @EnvironmentObject var settings: Settings
    var body: some View {
        Button("Theme: \(settings.theme)") { settings.toggle() }
    }
}
struct RootView: View {
    @StateObject var settings = Settings()
    var body: some View {
        ChildView().environmentObject(settings)
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
        let mut session = Session::new(&mut interp, "RootView").expect("session");

        let first = session.render().expect("render");
        assert!(
            uiir::to_json(&first).contains("Theme: dark"),
            "environment object injected: {}",
            uiir::to_json(&first)
        );

        // Tapping the child button toggles the injected object's @Published.
        let tap = Event {
            id: "0".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&tap).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains("Theme: light"),
            "mutation through the environment object is reflected: {}",
            uiir::to_json(&after)
        );
    }

    #[test]
    fn observed_object_shares_state_between_parent_and_child() {
        // A parent's `@StateObject` passed to a child's `@ObservedObject` is one
        // shared reference: a mutation from the child updates both views.
        let src = format!(
            "{PRELUDE}\n{}",
            r#"
class Model: ObservableObject {
    @Published var count = 0
    func bump() { count += 1 }
}
struct ChildView: View {
    @ObservedObject var model: Model
    var body: some View {
        Button("Child \(model.count)") { model.bump() }
    }
}
struct ParentView: View {
    @StateObject var model = Model()
    var body: some View {
        VStack {
            Text("Parent \(model.count)")
            ChildView(model: model)
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
        let mut session = Session::new(&mut interp, "ParentView").expect("session");
        session.render().expect("render");

        // Tap the child's button (id "0.1"); the shared model updates both.
        let tap = Event {
            id: "0.1".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&tap).expect("dispatch");
        let json = uiir::to_json(&after);
        assert!(
            json.contains("Parent 1"),
            "parent reflects shared model: {json}"
        );
        assert!(
            json.contains("Child 1"),
            "child reflects shared model: {json}"
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

    /// Build a session over `body` inside a `View` named `V` with the given
    /// stored `@State` declarations prepended.
    fn events_interp(program: &str) -> Interpreter<'static> {
        let src = format!("{PRELUDE}\n{program}");
        let analysis = tswift_frontend::Analysis::analyze(&src, "t.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let out: &'static mut std::io::Sink = Box::leak(Box::new(std::io::sink()));
        let mut interp = Interpreter::new(out);
        install(&mut interp);
        interp.run(analysis).expect("run");
        interp
    }

    #[test]
    fn on_tap_gesture_routes_a_tap_to_the_handler() {
        let mut interp = events_interp(
            r#"
struct TapView: View {
    @State private var count = 0
    var body: some View {
        VStack {
            Text("\(count)")
            Text("tap me").onTapGesture { count += 1 }
        }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "TapView").expect("session");
        let first = session.render().expect("render");
        let json = uiir::to_json(&first);
        // The tappable Text carries the marker modifier, no serialized closure.
        assert!(
            json.contains(r#"{"name":"onTapGesture","value":null}"#),
            "marker modifier present: {json}"
        );
        assert!(
            !json.contains("_handlers"),
            "handlers never serialize: {json}"
        );
        // Tapping the second child (id "0.1") bumps the counter.
        let tap = Event {
            id: "0.1".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&tap).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains(r#""verbatim":"1""#),
            "tap gesture bumps state: {}",
            uiir::to_json(&after)
        );
    }

    #[test]
    fn lifecycle_and_submit_events_route_to_handlers() {
        let mut interp = events_interp(
            r#"
struct LifecycleView: View {
    @State private var log = ""
    @State private var name = ""
    var body: some View {
        VStack {
            Text(log)
            TextField("Name", text: $name)
                .onSubmit { log = "submit" }
        }
        .onAppear { log = "appear" }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "LifecycleView").expect("session");
        session.render().expect("render");
        // The host fires `appear` on the root (id "0").
        let appear = Event {
            id: "0".into(),
            event: "appear".into(),
            value: None,
        };
        let after = session.dispatch(&appear).expect("dispatch");
        assert!(uiir::to_json(&after).contains("appear"), "onAppear fired");
        // Submitting the field (id "0.1") routes to its onSubmit handler.
        let submit = Event {
            id: "0.1".into(),
            event: "submit".into(),
            value: None,
        };
        let after = session.dispatch(&submit).expect("dispatch");
        assert!(uiir::to_json(&after).contains("submit"), "onSubmit fired");
    }

    #[test]
    fn on_change_fires_with_old_and_new_and_chains() {
        // Toggling `count` runs an onChange that mirrors it into `doubled`; a
        // second onChange watches `doubled` and records the transition. One
        // dispatch must cascade through both watchers before diffing.
        let mut interp = events_interp(
            r#"
struct ChangeView: View {
    @State private var count = 0
    @State private var doubled = 0
    @State private var log = ""
    var body: some View {
        VStack {
            Text("c=\(count) d=\(doubled) \(log)")
            Button("Inc") { count += 1 }
        }
        .onChange(of: count) { old, new in doubled = new * 2 }
        .onChange(of: doubled) { old, new in log = "\(old)->\(new)" }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "ChangeView").expect("session");
        session.render().expect("render");
        let tap = Event {
            id: "0.1".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&tap).expect("dispatch");
        let json = uiir::to_json(&after);
        assert!(
            json.contains("c=1 d=2 0-&gt;2") || json.contains("c=1 d=2 0->2"),
            "chained onChange should run both watchers with old/new: {json}"
        );
    }

    #[test]
    fn tabview_select_writes_tag_through_binding() {
        // `TabView(selection:)` with tagged tabs: a `select` event carrying a
        // tag writes it through the binding, so the `selection` arg follows.
        let mut interp = events_interp(
            r#"
struct TabsView: View {
    @State private var tab = "home"
    var body: some View {
        TabView(selection: $tab) {
            Text("Home").tabItem { Text("Home") }.tag("home")
            Text("Settings").tabItem { Text("Settings") }.tag("settings")
        }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "TabsView").expect("session");
        let first = session.render().expect("render");
        let json = uiir::to_json(&first);
        assert!(json.contains(r#""selection":"home""#), "initial: {json}");
        // The tab bar label + tag serialize as modifiers on each child.
        assert!(
            json.contains(r#"{"name":"tabItem","value":{"id":"0""#),
            "tabItem marker present: {json}"
        );

        let select = Event {
            id: "0".into(),
            event: "select".into(),
            value: Some(SwiftValue::Str("settings".into())),
        };
        let after = session.dispatch(&select).expect("dispatch");
        let json = uiir::to_json(&after);
        assert!(
            json.contains(r#""selection":"settings""#),
            "select writes tag through binding: {json}"
        );
    }

    #[test]
    fn tabview_select_without_binding_keeps_session_state() {
        // A `TabView` with no `selection:` binding defaults to its first tab
        // (index 0); a `select` event is remembered in per-node session state
        // and re-applied to the `selection` arg on the next render.
        let mut interp = events_interp(
            r#"
struct TabsView: View {
    var body: some View {
        TabView {
            Text("One").tabItem { Text("One") }
            Text("Two").tabItem { Text("Two") }
        }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "TabsView").expect("session");
        let first = session.render().expect("render");
        assert!(
            uiir::to_json(&first).contains(r#""selection":0"#),
            "defaults to index 0: {}",
            uiir::to_json(&first)
        );

        let select = Event {
            id: "0".into(),
            event: "select".into(),
            value: Some(SwiftValue::int(1)),
        };
        let after = session.dispatch(&select).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains(r#""selection":1"#),
            "session keeps per-node selection: {}",
            uiir::to_json(&after)
        );
        // The stored selection survives a subsequent unrelated re-render.
        let again = session.render().expect("render");
        assert!(uiir::to_json(&again).contains(r#""selection":1"#));
    }

    #[test]
    fn navigation_link_tap_pushes_and_back_pops() {
        // A `NavigationStack` with a `NavigationLink`: tapping the link pushes
        // its destination as a second child; `back` pops it. `back` on the
        // single-child (root-only) stack is a no-op.
        let mut interp = events_interp(
            r#"
struct RootView: View {
    var body: some View {
        NavigationStack {
            NavigationLink("Go") {
                Text("Detail").navigationTitle("Detail")
            }
        }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "RootView").expect("session");
        let first = session.render().expect("render");
        let json = uiir::to_json(&first);
        // Root stack has exactly one child (the link); no destination leaks.
        assert!(json.contains(r#""kind":"NavigationStack""#), "{json}");
        assert!(json.contains(r#""kind":"NavigationLink""#), "{json}");
        assert!(
            !json.contains("_destination"),
            "destination never serialized: {json}"
        );
        assert!(
            !json.contains("Detail"),
            "destination subtree not serialized: {json}"
        );

        // A `back` before any push is a no-op (still one child).
        let back = Event {
            id: "0".into(),
            event: "back".into(),
            value: None,
        };
        let after = session.dispatch(&back).expect("dispatch");
        assert!(
            !uiir::to_json(&after).contains("Detail"),
            "back on root is a no-op"
        );

        // Tap the link (id "0.0") → the detail screen is pushed as a second child.
        let tap = Event {
            id: "0.0".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&tap).expect("dispatch");
        let json = uiir::to_json(&after);
        assert!(
            json.contains("Detail"),
            "link tap pushes the destination: {json}"
        );
        assert!(
            json.contains(r#""name":"navigationTitle","value":"Detail""#),
            "pushed screen carries its navigationTitle: {json}"
        );

        // `back` pops the pushed screen.
        let after = session.dispatch(&back).expect("dispatch");
        assert!(
            !uiir::to_json(&after).contains("Detail"),
            "back pops the pushed screen: {}",
            uiir::to_json(&after)
        );
    }

    #[test]
    fn pushed_screen_re_renders_against_state_change() {
        // A pushed destination reads the root's `@State`; a mutation on the root
        // (still live in the tree) is reflected on the pushed screen's next
        // render — proving pushed closures re-evaluate fresh (not snapshots).
        let mut interp = events_interp(
            r#"
struct RootView: View {
    @State private var count = 0
    var body: some View {
        NavigationStack {
            VStack {
                Button("Inc") { count += 1 }
                NavigationLink("Go") {
                    Text("Count: \(count)")
                }
            }
        }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "RootView").expect("session");
        session.render().expect("render");
        // Push the detail (link id "0.0.1": stack > VStack(0.0) > link(0.0.1)).
        let tap = Event {
            id: "0.0.1".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&tap).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains("Count: 0"),
            "pushed: {}",
            uiir::to_json(&after)
        );
        // Increment the root counter (button id "0.0.0"); the pushed screen,
        // re-evaluated fresh, reflects the new value.
        let inc = Event {
            id: "0.0.0".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&inc).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains("Count: 1"),
            "pushed screen re-reads @State: {}",
            uiir::to_json(&after)
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
