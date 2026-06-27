//! The diff engine — two UIIR trees → a keyed patch stream (plan §3.2).
//!
//! Nodes are matched by id, so a changed `Text` emits a `setText` fast-path, an
//! arg change emits `setArgs`, a modifier-list change emits `setModifiers`, a
//! kind change emits `replace`, and child-count changes emit `insert`/`remove`.
//!
//! Children are reconciled positionally by default. A `ForEach` node instead
//! reconciles its children by their stable identity key (Tier 3): rows present
//! in both trees are matched by key (so their subtree is diffed in place),
//! removed keys emit `remove`, new keys emit `insert`, and a key whose relative
//! order changed emits `move`.

use std::collections::HashMap;

use tswift_core::SwiftValue;

use crate::{key_of, uiir, CHILDREN_FIELD, MODIFIERS_FIELD};

/// One patch operation against the host tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Patch {
    /// Initial render: insert the whole subtree into an empty host.
    Mount { node: SwiftValue },
    /// Insert `node` (carrying its subtree) as child `index` of `parent`.
    Insert {
        parent: String,
        index: usize,
        node: SwiftValue,
    },
    /// Remove the node at `id` (and its subtree).
    Remove { id: String },
    /// Replace the node at `id` wholesale (kind changed).
    Replace { id: String, node: SwiftValue },
    /// `Text` fast-path: set the node's verbatim string.
    SetText { id: String, text: String },
    /// Replace the node's whole ordered modifier list.
    SetModifiers {
        id: String,
        modifiers: Vec<SwiftValue>,
    },
    /// Replace the node's constructor args.
    SetArgs { id: String, node: SwiftValue },
    /// Move an existing keyed child to `index` within `parent` (keyed reorder).
    Move {
        parent: String,
        id: String,
        index: usize,
    },
}

/// The kind (SwiftUI type name) of a view node.
fn kind(node: &SwiftValue) -> &str {
    match node {
        SwiftValue::Struct(obj) => obj.type_name.as_str(),
        _ => "",
    }
}

/// The ordered modifier list of a node.
fn modifiers(node: &SwiftValue) -> Vec<SwiftValue> {
    match node {
        SwiftValue::Struct(obj) => match obj.get(MODIFIERS_FIELD) {
            Some(SwiftValue::Array(items)) => items.iter().cloned().collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// The ordered child views of a node.
fn children(node: &SwiftValue) -> Vec<SwiftValue> {
    match node {
        SwiftValue::Struct(obj) => match obj.get(CHILDREN_FIELD) {
            Some(SwiftValue::Array(items)) => items.iter().cloned().collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// A node's `Text` verbatim string, if it is a `Text`.
fn text(node: &SwiftValue) -> Option<String> {
    match node {
        SwiftValue::Struct(obj) if obj.type_name == "Text" => match obj.get("verbatim") {
            Some(SwiftValue::Str(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Whether two nodes' visible constructor args (excluding internal fields and
/// modifiers/children) are equal.
fn args_equal(a: &SwiftValue, b: &SwiftValue) -> bool {
    fn visible(node: &SwiftValue) -> Vec<(String, SwiftValue)> {
        match node {
            SwiftValue::Struct(obj) => obj
                .fields
                .iter()
                .filter(|(k, _)| !k.starts_with('_'))
                .cloned()
                .collect(),
            _ => Vec::new(),
        }
    }
    visible(a) == visible(b)
}

/// The initial mount patch for a freshly rendered tree.
pub fn mount(tree: &SwiftValue) -> Vec<Patch> {
    vec![Patch::Mount { node: tree.clone() }]
}

/// Diff `old` into `new`, emitting the patch stream that transforms the host
/// tree rooted at `old` into `new`.
pub fn diff(old: &SwiftValue, new: &SwiftValue) -> Vec<Patch> {
    let mut patches = Vec::new();
    diff_node(old, new, "0", &mut patches);
    patches
}

fn diff_node(old: &SwiftValue, new: &SwiftValue, id: &str, patches: &mut Vec<Patch>) {
    // A kind change can't be patched incrementally — replace wholesale.
    if kind(old) != kind(new) {
        patches.push(Patch::Replace {
            id: id.to_string(),
            node: new.clone(),
        });
        return;
    }

    // Args: a `Text` change uses the fast-path; any other arg change replaces
    // the node's args.
    match (text(old), text(new)) {
        (Some(a), Some(b)) if a != b => patches.push(Patch::SetText {
            id: id.to_string(),
            text: b,
        }),
        _ if !args_equal(old, new) => patches.push(Patch::SetArgs {
            id: id.to_string(),
            node: new.clone(),
        }),
        _ => {}
    }

    // Modifiers: whole-list replacement when the ordered list differs.
    let (old_mods, new_mods) = (modifiers(old), modifiers(new));
    if old_mods != new_mods {
        patches.push(Patch::SetModifiers {
            id: id.to_string(),
            modifiers: new_mods,
        });
    }

    // Children: keyed reconciliation when the rows carry stable identity keys
    // (`ForEach`, and the `List(_:id:)` shorthand), positional otherwise. Both
    // sides must be fully keyed so a real key (e.g. `"0"`) can never be matched
    // against an unkeyed child's structural index of the same spelling — a
    // `List` toggling between static and data-driven forms falls to positional.
    let (old_children, new_children) = (children(old), children(new));
    if all_keyed(&old_children) && all_keyed(&new_children) {
        diff_keyed_children(&old_children, &new_children, id, patches);
    } else {
        diff_positional_children(&old_children, &new_children, id, patches);
    }
}

/// Whether every child carries a stable identity key (vacuously true for an
/// empty list, so an emptied or freshly populated keyed list stays on the
/// keyed path).
fn all_keyed(children: &[SwiftValue]) -> bool {
    children.iter().all(|c| key_of(c).is_some())
}

/// Positional child diff: match children index-for-index, then insert/remove the
/// tail. Stable for static containers whose child identities are their order.
fn diff_positional_children(
    old_children: &[SwiftValue],
    new_children: &[SwiftValue],
    id: &str,
    patches: &mut Vec<Patch>,
) {
    let common = old_children.len().min(new_children.len());
    for i in 0..common {
        diff_node(
            &old_children[i],
            &new_children[i],
            &format!("{id}.{i}"),
            patches,
        );
    }
    for (i, child) in new_children.iter().enumerate().skip(common) {
        patches.push(Patch::Insert {
            parent: id.to_string(),
            index: i,
            node: child.clone(),
        });
    }
    // Remove trailing old children high-index-first so earlier ids stay valid.
    for i in (common..old_children.len()).rev() {
        patches.push(Patch::Remove {
            id: format!("{id}.{i}"),
        });
    }
}

/// Keyed child diff for `ForEach`: rows carry a stable identity key, so reorders
/// preserve element identity (DOM node, its state) via `move` instead of
/// rebuilding. Removed keys are dropped, surviving keys are diffed in place, new
/// keys are inserted, and a `move` is emitted whenever a surviving key's target
/// position differs from where the running host order would otherwise place it.
fn diff_keyed_children(
    old_children: &[SwiftValue],
    new_children: &[SwiftValue],
    id: &str,
    patches: &mut Vec<Patch>,
) {
    // Map each old child's key to its node so survivors can be diffed in place.
    let old_by_key: HashMap<String, &SwiftValue> = old_children
        .iter()
        .enumerate()
        .map(|(i, c)| (child_key(c, i), c))
        .collect();
    let new_keys: Vec<String> = new_children
        .iter()
        .enumerate()
        .map(|(i, c)| child_key(c, i))
        .collect();
    let new_key_set: std::collections::HashSet<&String> = new_keys.iter().collect();

    // Remove keys that disappeared.
    for (i, child) in old_children.iter().enumerate() {
        let key = child_key(child, i);
        if !new_key_set.contains(&key) {
            patches.push(Patch::Remove {
                id: format!("{id}.{key}"),
            });
        }
    }

    // `order` tracks the host's current child order (keys) as we apply patches,
    // so a `move` is only emitted when an element is genuinely out of place.
    let mut order: Vec<String> = old_children
        .iter()
        .enumerate()
        .map(|(i, c)| child_key(c, i))
        .filter(|k| new_key_set.contains(k))
        .collect();

    for (target, new_child) in new_children.iter().enumerate() {
        let key = &new_keys[target];
        let child_path = format!("{id}.{key}");
        match old_by_key.get(key) {
            Some(old_child) => {
                diff_node(old_child, new_child, &child_path, patches);
                let cur = order.iter().position(|k| k == key).unwrap();
                if cur != target {
                    let item = order.remove(cur);
                    order.insert(target, item);
                    patches.push(Patch::Move {
                        parent: id.to_string(),
                        id: child_path,
                        index: target,
                    });
                }
            }
            None => {
                patches.push(Patch::Insert {
                    parent: id.to_string(),
                    index: target,
                    node: new_child.clone(),
                });
                order.insert(target, key.clone());
            }
        }
    }
}

/// The identity key of a keyed child, or its structural index as a fallback (so
/// a non-keyed child under a `ForEach` still gets a stable-enough path).
fn child_key(child: &SwiftValue, index: usize) -> String {
    key_of(child)
        .map(str::to_string)
        .unwrap_or_else(|| index.to_string())
}

/// Serialize a patch stream as a canonical JSON array (the Layer-C wire format).
pub fn to_json(patches: &[Patch]) -> String {
    let mut out = String::new();
    out.push('[');
    for (i, p) in patches.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&patch_json(p));
    }
    out.push(']');
    out
}

fn patch_json(patch: &Patch) -> String {
    match patch {
        Patch::Mount { node } => {
            format!(r#"{{"op":"mount","node":{}}}"#, uiir::node_json(node, "0"))
        }
        Patch::Insert {
            parent,
            index,
            node,
        } => format!(
            r#"{{"op":"insert","parentId":{},"index":{},"node":{}}}"#,
            json_string(parent),
            index,
            // A keyed (`ForEach`) child keeps its stable id under insertion so
            // later move/remove/setText/events for `{parent}.{key}` resolve.
            uiir::node_json(node, &crate::child_id(parent, *index, node)),
        ),
        Patch::Remove { id } => format!(r#"{{"op":"remove","id":{}}}"#, json_string(id)),
        Patch::Replace { id, node } => format!(
            r#"{{"op":"replace","id":{},"node":{}}}"#,
            json_string(id),
            uiir::node_json(node, id),
        ),
        Patch::SetText { id, text } => format!(
            r#"{{"op":"setText","id":{},"text":{}}}"#,
            json_string(id),
            json_string(text),
        ),
        Patch::SetModifiers { id, modifiers } => format!(
            r#"{{"op":"setModifiers","id":{},"modifiers":{}}}"#,
            json_string(id),
            uiir::modifiers_json(modifiers),
        ),
        Patch::SetArgs { id, node } => format!(
            r#"{{"op":"setArgs","id":{},"args":{}}}"#,
            json_string(id),
            uiir::args_json(node),
        ),
        Patch::Move { parent, id, index } => format!(
            r#"{{"op":"move","parentId":{},"id":{},"index":{}}}"#,
            json_string(parent),
            json_string(id),
            index,
        ),
    }
}

/// Minimal JSON string escaping (mirrors `uiir`).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Event, Session};
    use crate::{install, PRELUDE};
    use tswift_core::Interpreter;

    fn counter_session_json() -> String {
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
        let interp: &'static mut Interpreter<'static> = Box::leak(Box::new(interp));
        let mut session = Session::new(interp, "CounterView").expect("session");

        let first = session.render().expect("render");
        let mount_patches = mount(&first);
        let tap = Event {
            id: "0.1".into(),
            event: "tap".into(),
            value: None,
        };
        let before = session.current_tree().expect("tree").clone();
        let after = session.dispatch(&tap).expect("dispatch");
        let patches = diff(&before, &after);
        format!("{}\n{}", to_json(&mount_patches), to_json(&patches))
    }

    #[test]
    fn counter_tap_emits_set_text_patch() {
        let json = counter_session_json();
        let lines: Vec<&str> = json.lines().collect();
        // The mount carries the whole VStack subtree.
        assert!(lines[0].starts_with(r#"[{"op":"mount""#));
        // The tap changes only the Text child "0.0" from "0" to "1".
        assert_eq!(lines[1], r#"[{"op":"setText","id":"0.0","text":"1"}]"#);
    }

    /// Render a `ForEach` over a literal string array (id: \.self), each row a
    /// `Text(name)`, and return the root view value.
    fn render_foreach(items: &[&str]) -> SwiftValue {
        let literal = items
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let src = format!(
            "{PRELUDE}\nstruct V: View {{ var body: some View {{ \
             ForEach([{literal}], id: \\.self) {{ name in Text(name) }} }} }}\n"
        );
        let analysis = tswift_frontend::Analysis::analyze(&src, "t.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        install(&mut interp);
        interp.run(analysis).expect("run");
        crate::render_root(&mut interp, "V").expect("render")
    }

    #[test]
    fn foreach_reorder_emits_only_moves() {
        let before = render_foreach(&["a", "b", "c"]);
        let after = render_foreach(&["c", "a", "b"]);
        let patches = diff(&before, &after);
        // No rebuild: every patch is a `move` (rows kept their identity).
        assert!(
            patches.iter().all(|p| matches!(p, Patch::Move { .. })),
            "expected only moves, got {patches:?}"
        );
        // Moving `c` to the front is the single minimal reorder.
        assert_eq!(
            patches,
            vec![Patch::Move {
                parent: "0".into(),
                id: "0.c".into(),
                index: 0,
            }]
        );
    }

    #[test]
    fn foreach_insert_and_remove_use_keyed_ids() {
        let before = render_foreach(&["a", "b", "c"]);
        let after = render_foreach(&["a", "x", "c"]);
        let patches = diff(&before, &after);
        // `b` removed by key, `x` inserted at its position — no `c` churn.
        assert!(patches.contains(&Patch::Remove { id: "0.b".into() }));
        assert!(patches.iter().any(
            |p| matches!(p, Patch::Insert { parent, index, .. } if parent == "0" && *index == 1)
        ));
        assert!(!patches.iter().any(|p| matches!(p, Patch::Replace { .. })));
    }

    #[test]
    fn keyed_insert_serializes_with_the_row_key_not_its_index() {
        // Inserting a new key registers the subtree under `{parent}.{key}` so
        // later move/remove/setText/events for that row resolve in the host.
        let before = render_foreach(&["a", "c"]);
        let after = render_foreach(&["a", "b", "c"]);
        let json = to_json(&diff(&before, &after));
        assert!(
            json.contains(r#""op":"insert","parentId":"0","index":1,"node":{"id":"0.b""#),
            "insert subtree must use the keyed id: {json}"
        );
    }

    /// Render `List(items, id: \.self) { Text($0) }` for the given items.
    fn render_list(items: &[&str]) -> SwiftValue {
        let literal = items
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let src = format!(
            "{PRELUDE}\nstruct V: View {{ var body: some View {{ \
             List([{literal}], id: \\.self) {{ name in Text(name) }} }} }}\n"
        );
        let analysis = tswift_frontend::Analysis::analyze(&src, "t.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        install(&mut interp);
        interp.run(analysis).expect("run");
        crate::render_root(&mut interp, "V").expect("render")
    }

    #[test]
    fn list_data_shorthand_reorder_emits_moves() {
        // The generalized keyed diff must also `move` rows for `List(_:id:)`,
        // whose keyed children are direct (no wrapping `ForEach` node).
        let before = render_list(&["a", "b", "c"]);
        let after = render_list(&["c", "a", "b"]);
        let patches = diff(&before, &after);
        assert_eq!(
            patches,
            vec![Patch::Move {
                parent: "0".into(),
                id: "0.c".into(),
                index: 0,
            }]
        );
    }

    #[test]
    fn foreach_identical_data_emits_no_patches() {
        // Stable keys + unchanged content reconcile to nothing.
        let before = render_foreach(&["a", "b"]);
        let after = render_foreach(&["a", "b"]);
        assert!(diff(&before, &after).is_empty());
    }
}
