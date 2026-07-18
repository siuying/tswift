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

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use tswift_core::{EvalError, Interpreter, StdContext, StructObj, SwiftValue};

use crate::views::SCENE_CONTENT_FIELD;
use crate::{
    BINDING_FIELD, CHILDREN_FIELD, HANDLERS_FIELD, HANDLERS_TYPE, MODIFIERS_FIELD,
    NAV_DESTINATIONS_FIELD, NAV_DESTINATION_FIELD, NAV_VALUE_FIELD, PRESENTATIONS_FIELD,
    PRESENTATION_NODE_KIND, WATCH_FIELD,
};

/// Maximum `onChange` cascade passes per dispatch: a watcher's action may mutate
/// state a *second* watcher observes (chained state). We re-render and re-scan
/// until quiescent, bounded so a watcher toggling its own source cannot hang.
const MAX_WATCH_PASSES: usize = 64;

/// Extract the display string of a realized `Text` view (an alert/dialog
/// `message:` closure resolves to one). Reads the `verbatim` or `key` arg;
/// returns `None` for a non-`Text` value.
fn message_text(view: &SwiftValue) -> Option<String> {
    let SwiftValue::Struct(obj) = view else {
        return None;
    };
    if obj.type_name != "Text" {
        return None;
    }
    for key in ["verbatim", "key"] {
        if let Some(SwiftValue::Str(s)) = obj.get(key) {
            return Some(s.clone());
        }
    }
    None
}

/// Whether a presentation's gating value is "open": a `Bool(true)`
/// (`isPresented:`) or any non-`nil` value (`item:` presentations). Everything
/// else (`Bool(false)`, `Nil`) is closed.
fn is_presented(value: &SwiftValue) -> bool {
    match value {
        SwiftValue::Bool(b) => *b,
        SwiftValue::Nil => false,
        _ => true,
    }
}

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

/// Identity passed to `task(id:)`, if present on this view. Plain `.task` uses
/// `Void` as a stable per-mounted-node sentinel. Modifiers are retained in
/// source order, so the final task marker is the one paired with the handler
/// slot (later `.task` calls replace the same `"task"` handler key).
fn task_identity(obj: &StructObj) -> SwiftValue {
    let Some(SwiftValue::Array(modifiers)) = obj.get(MODIFIERS_FIELD) else {
        return SwiftValue::Void;
    };
    modifiers
        .iter()
        .rev()
        .find_map(|modifier| match modifier {
            SwiftValue::Struct(record)
                if matches!(record.get("name"), Some(SwiftValue::Str(name)) if name == "task") =>
            {
                Some(record.get("id").cloned().unwrap_or(SwiftValue::Void))
            }
            _ => None,
        })
        .unwrap_or(SwiftValue::Void)
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
    entry: SessionEntry,
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
    /// Per-`AsyncImage` phase state (ADR-0013 §4), keyed by the node's
    /// structural id. Each entry is `(phase, url)` where `phase` is
    /// `"empty"` | `"success"` | `"failure"` and `url` is the URL at the time
    /// of the last `imagePhase` event (used to detect URL changes, which reset
    /// the phase back to `"empty"`).
    image_phase: HashMap<String, (String, String)>,
    /// The last identity value whose `.task` ran for each mounted node. A node
    /// without `task(id:)` uses `Void`, so it runs once per mount; changing an
    /// explicit id schedules one new run after the render that observed it.
    task_ids: HashMap<String, SwiftValue>,
    /// Whether the host has completed its first lifecycle pass. Until then a
    /// dispatch must not eagerly fire `.task` merely because a fixture/event
    /// arrived before mount; after mount it lets `task(id:)` re-run on changes.
    task_lifecycle_started: bool,
}

/// The retained value a session re-evaluates. App scenes are unwrapped once so
/// the session keeps the root view instance (and therefore its `@State`) just
/// like the legacy root-view path.
enum SessionEntry {
    View(SwiftValue),
    App {
        root: SwiftValue,
        scopes: Vec<SwiftValue>,
    },
}

/// Descend a headless App scene while retaining every wrapper whose render
/// scope must be restored around the root view on each session render.
fn extract_scene_root(
    interp: &mut Interpreter<'_>,
    scene: SwiftValue,
    scopes: &mut Vec<SwiftValue>,
    roots: &mut Vec<(SwiftValue, Vec<SwiftValue>)>,
) -> Result<(), EvalError> {
    let SwiftValue::Struct(obj) = &scene else {
        match scene {
            SwiftValue::Array(items) => {
                for item in items.iter() {
                    extract_scene_root(interp, item.clone(), scopes, roots)?;
                }
                return Ok(());
            }
            SwiftValue::Tuple(items, _) => {
                for item in items {
                    extract_scene_root(interp, item, scopes, roots)?;
                }
                return Ok(());
            }
            _ => {}
        }
        return Err(EvalError::Unsupported(format!(
            "headless SwiftUI App scene must contain WindowGroup, found `{}`",
            scene.type_name()
        )));
    };

    let type_name = obj.type_name.clone();
    let is_window_group = type_name == "WindowGroup";
    let is_group = type_name == "Group";
    let is_builtin_view = obj.fields.iter().any(|(name, _)| name == MODIFIERS_FIELD);
    scopes.push(scene.clone());
    interp.view_scope_enter(&scene);
    let result = (|| -> Result<(), EvalError> {
        if is_window_group {
            let content = obj
                .get(SCENE_CONTENT_FIELD)
                .cloned()
                .ok_or_else(|| EvalError::Trap("WindowGroup is missing scene content".into()))?;
            let SwiftValue::Closure(id) = content else {
                return Err(EvalError::Trap(
                    "WindowGroup scene content is not a closure".into(),
                ));
            };
            let built = interp
                .eval_block_values(id)
                .map_err(crate::std_error_to_eval)?;
            let SwiftValue::Array(items) = built else {
                return Err(EvalError::Trap(
                    "WindowGroup builder did not produce children".into(),
                ));
            };
            if items.len() != 1 {
                return Err(EvalError::Unsupported(format!(
                "headless WindowGroup requires one root View, found {}; multiple root views are unsupported",
                items.len()
            )));
            }
            roots.push((items[0].clone(), scopes.clone()));
            Ok(())
        } else if is_group {
            let children = obj
                .get(CHILDREN_FIELD)
                .cloned()
                .unwrap_or_else(|| SwiftValue::Array(Rc::new(Vec::new())));
            extract_scene_root(interp, children, scopes, roots)
        } else if !is_builtin_view {
            let body = interp.get_member(&scene, "body")?;
            extract_scene_root(interp, body, scopes, roots)
        } else {
            Err(EvalError::Unsupported(format!(
                "headless SwiftUI App scene `{type_name}` is unsupported; use WindowGroup"
            )))
        }
    })();
    interp.view_scope_exit(&scene);
    scopes.pop();
    result
}

impl<'i, 'w> Session<'i, 'w> {
    /// Instantiate `root_type` once and start a session over it.
    pub fn new(interp: &'i mut Interpreter<'w>, root_type: &str) -> Result<Self, EvalError> {
        let instance = interp.make_struct(root_type, &[])?;
        Ok(Session {
            interp,
            entry: SessionEntry::View(instance),
            current: None,
            tab_selection: HashMap::new(),
            nav_stack: HashMap::new(),
            image_phase: HashMap::new(),
            task_ids: HashMap::new(),
            task_lifecycle_started: false,
        })
    }

    /// Start a session from a SwiftUI `App`. The host accepts one `WindowGroup`
    /// (possibly nested in simple `Scene`/`Group` composition) and exposes its
    /// root `View` to the existing render/session path. Multiple windows and
    /// platform-owned scenes deliberately fail rather than pretending to have a
    /// lifecycle the headless host cannot provide.
    pub fn new_app(interp: &'i mut Interpreter<'w>, app_type: &str) -> Result<Self, EvalError> {
        let app = interp.make_struct(app_type, &[])?;
        let scene = SwiftValue::Array(Rc::new(interp.get_member_values(&app, "body")?));
        let mut roots = Vec::new();
        extract_scene_root(interp, scene, &mut Vec::new(), &mut roots)?;
        let [(root, scopes)] = roots.as_slice() else {
            return Err(EvalError::Unsupported(match roots.len() {
                0 => {
                    format!("headless SwiftUI App entry `{app_type}` requires a WindowGroup scene")
                }
                _ => "multiple windows".into(),
            }));
        };
        Ok(Session {
            interp,
            entry: SessionEntry::App {
                root: root.clone(),
                scopes: scopes.clone(),
            },
            current: None,
            tab_selection: HashMap::new(),
            nav_stack: HashMap::new(),
            image_phase: HashMap::new(),
            task_ids: HashMap::new(),
            task_lifecycle_started: false,
        })
    }

    /// Run the interpreter's teardown finalizers (closing any framework-held
    /// native resources, e.g. open SwiftData database handles) ahead of the
    /// session being dropped. Idempotent — `Interpreter::teardown` drains its
    /// finalizer list. A caller that leaks the interpreter (so its `Drop` never
    /// runs) must call this when discarding a session to avoid leaking those
    /// native resources.
    pub fn teardown(&mut self) {
        self.interp.teardown();
    }

    /// Evaluate the root view's `body` into a fresh UIIR tree, caching it for
    /// event routing.
    pub fn render(&mut self) -> Result<SwiftValue, EvalError> {
        let tree = match &self.entry {
            SessionEntry::View(instance) => {
                let body = self.interp.get_member(instance, "body")?;
                crate::resolve_root(self.interp, body).map_err(crate::std_error_to_eval)?
            }
            SessionEntry::App { root, scopes } => {
                for scope in scopes {
                    self.interp.view_scope_enter(scope);
                }
                let tree = crate::resolve_root(self.interp, root.clone())
                    .map_err(crate::std_error_to_eval);
                for scope in scopes.iter().rev() {
                    self.interp.view_scope_exit(scope);
                }
                tree?
            }
        };
        // Three separate full-tree rewrites, deliberately not merged into one
        // pass: the nav pass appends pushed screens as new children which the
        // tab and image passes must still visit (session tab selection or
        // image phase inside a pushed screen), and a single post-order pass
        // never re-walks children a node transform appends.
        //
        // 1. Append each `NavigationStack`'s pushed screens (ADR-0013 §1).
        let tree = crate::tree::rewrite(tree, "0", &mut |obj, id| self.nav_stack_node(obj, id))?;
        // 1b. Realize each open presentation (`.sheet`/`.fullScreenCover`/
        // `.popover`) into a `Presentation` child node (ADR-0019). Runs after
        // the nav pass so a presentation inside a pushed screen is visited, and
        // before the tab/image passes so a sheet's content (which may hold a
        // `TabView`/`AsyncImage`) is still walked.
        let tree = crate::tree::rewrite(tree, "0", &mut |obj, _| self.presentation_node(obj))?;
        // 2. Re-apply any per-node `TabView` selection owned by the session
        // (the no-binding case): the freshly evaluated `body` defaults each
        // such tab view to its first tab, so the stored selection overrides it.
        let tree =
            crate::tree::rewrite(tree, "0", &mut |obj, id| self.tab_selection_node(obj, id))?;
        // 3. Resolve each `AsyncImage` node's phase-appropriate children
        // (ADR-0013 §4): evaluate content/placeholder/phase closures from
        // session state.
        let tree = crate::tree::rewrite(tree, "0", &mut |obj, id| self.image_phase_node(obj, id))?;
        // Prune phase state for node ids that no longer appear in the tree so
        // the map does not accumulate stale entries across renders (Fix #3).
        self.prune_image_phase(&tree);
        self.current = Some(tree.clone());
        Ok(tree)
    }

    /// Resolve an `AsyncImage` node's phase-appropriate children (ADR-0013
    /// §4). For each `AsyncImage` node that has closure fields
    /// (`_asyncContent`, `_asyncPlaceholder`, or `_asyncPhaseContent`),
    /// evaluates the appropriate closure based on the session’s stored phase
    /// (keyed by the node’s structural id, default `"empty"`) and updates the
    /// node’s `_children` and `phase` arg. URL changes (URL arg differs from
    /// the stored phase’s URL) reset the phase back to `"empty"`. Any other
    /// node passes through untouched.
    fn image_phase_node(&mut self, obj: &mut StructObj, id: &str) -> Result<(), EvalError> {
        // Only act on AsyncImage nodes that have captured closures (v1.5).
        if obj.type_name != "AsyncImage" || !crate::has_async_image_closures(obj) {
            return Ok(());
        }
        // Retrieve the current URL from the node’s args.
        let url = match obj.get("url") {
            Some(SwiftValue::Str(s)) => s.clone(),
            _ => String::new(),
        };

        // Determine the effective phase: use stored state when the URL
        // hasn’t changed; reset to `"empty"` on URL change.
        let effective_phase: String = match self.image_phase.get(id) {
            Some((phase, stored_url)) if stored_url == &url => phase.clone(),
            Some(_) => {
                // URL changed — reset phase so the host re-fires load events.
                self.image_phase.remove(id);
                "empty".to_string()
            }
            None => "empty".to_string(),
        };

        // Update the serialized `phase` arg.
        if let Some(slot) = obj.fields.iter_mut().find(|(k, _)| k == "phase") {
            slot.1 = SwiftValue::Str(effective_phase.clone());
        }

        // Evaluate the closure and inject the result as children.
        let node_val = SwiftValue::Struct(Rc::new(obj.clone()));
        let child =
            crate::realize_async_image_child(self.interp, &node_val, &effective_phase, &url)
                .map_err(crate::std_error_to_eval)?;

        let new_children = match child {
            Some(c) => vec![c],
            None => Vec::new(),
        };
        if let Some(pos) = obj.fields.iter().position(|(k, _)| k == CHILDREN_FIELD) {
            obj.fields[pos].1 = SwiftValue::Array(Rc::new(new_children));
        } else {
            obj.fields.push((
                CHILDREN_FIELD.into(),
                SwiftValue::Array(Rc::new(new_children)),
            ));
        }
        Ok(())
    }

    /// Remove entries from `image_phase` whose structural ids no longer appear
    /// in `tree`. Called after each render so the map does not accumulate
    /// phantom entries for nodes that were removed from the view tree (e.g. an
    /// `AsyncImage` inside a pushed screen that was popped).
    fn prune_image_phase(&mut self, tree: &SwiftValue) {
        if self.image_phase.is_empty() {
            return;
        }
        let live = collect_async_image_ids(tree);
        self.image_phase.retain(|id, _| live.contains(id));
    }

    /// Override the `selection` arg of a `TabView` node lacking a `selection:`
    /// binding with the session's stored per-node selection (if any). Bound tab
    /// views read their selection from the binding, so they are left untouched,
    /// as is every other node.
    fn tab_selection_node(&self, obj: &mut StructObj, id: &str) -> Result<(), EvalError> {
        if obj.type_name == "TabView" && !obj.fields.iter().any(|(k, _)| k == BINDING_FIELD) {
            if let Some(selected) = self.tab_selection.get(id) {
                if let Some(slot) = obj.fields.iter_mut().find(|(k, _)| k == "selection") {
                    slot.1 = selected.clone();
                }
            }
        }
        Ok(())
    }

    /// Append a `NavigationStack` node's pushed screens (ADR-0013 §1) as
    /// ordinary children, keyed by the stack's structural id. A path-bound
    /// stack (`NavigationStack(path:)`) derives them from the bound path's
    /// items, each matched to a `.navigationDestination(for:)` registration in
    /// the stack's subtree; a session-owned stack uses the per-stack
    /// `nav_stack` state. Each pushed destination is realized fresh — a
    /// `@ViewBuilder` closure is re-evaluated so the screen re-reads `@State`;
    /// an eager destination view is expanded as-is — and appended after the
    /// stack's root content (root first, topmost last). Non-stack nodes pass
    /// through untouched.
    fn nav_stack_node(&mut self, obj: &mut StructObj, id: &str) -> Result<(), EvalError> {
        if obj.type_name != "NavigationStack" {
            return Ok(());
        }
        let screens = if let Some(binding) = obj.get(BINDING_FIELD).cloned() {
            self.realize_path_screens(obj, &binding)?
        } else {
            self.realize_session_screens(id)?
        };
        if screens.is_empty() {
            return Ok(());
        }
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
        kids.extend(screens);
        obj.fields[pos].1 = SwiftValue::Array(Rc::new(kids));
        Ok(())
    }

    /// Realize a node's open presentations (ADR-0019). For each
    /// [`PRESENTATIONS_FIELD`] record whose gating `Binding` reads truthy
    /// (`Bool(true)`, or a non-`nil` `item:` value), evaluate the deferred
    /// `@ViewBuilder` content closure fresh and append a `Presentation` node as
    /// the presenting node's last child. The `Presentation` node carries the
    /// gating binding (`_binding`) and an optional `dismiss` handler
    /// (`_handlers`) so a host `dismiss` event can write back and fire
    /// `onDismiss`. A closed presentation contributes nothing (the diff removes
    /// a previously-open node), and programmatic close (the bound state going
    /// `false`) drops it on the next render for free.
    fn presentation_node(&mut self, obj: &mut StructObj) -> Result<(), EvalError> {
        let Some(SwiftValue::Array(records)) = obj.get(PRESENTATIONS_FIELD).cloned() else {
            return Ok(());
        };
        let mut nodes: Vec<SwiftValue> = Vec::new();
        for record in records.iter() {
            let SwiftValue::Struct(rec) = record else {
                continue;
            };
            let Some(binding) = rec.get(BINDING_FIELD).cloned() else {
                continue;
            };
            let wrapped = self.interp.get_member(&binding, "wrappedValue")?;
            if !is_presented(&wrapped) {
                continue;
            }
            let Some(SwiftValue::Closure(cid)) = rec.get("_content").cloned() else {
                continue;
            };
            let content = crate::realize_pushed_screen(self.interp, &SwiftValue::Closure(cid))
                .map_err(crate::std_error_to_eval)?;
            let style = match rec.get("style") {
                Some(SwiftValue::Str(s)) => s.clone(),
                _ => "sheet".to_string(),
            };
            let mut arg_fields: Vec<(String, SwiftValue)> =
                vec![("style".into(), SwiftValue::Str(style))];
            // `alert`/`confirmationDialog` carry a `title` arg and an optional
            // `message` arg (the `message:` closure evaluated to its text).
            if let Some(SwiftValue::Str(t)) = rec.get("title") {
                arg_fields.push(("title".into(), SwiftValue::Str(t.clone())));
            }
            if let Some(SwiftValue::Closure(mid)) = rec.get("_message") {
                if let Some(msg) =
                    crate::realize_pushed_screen(self.interp, &SwiftValue::Closure(*mid))
                        .map_err(crate::std_error_to_eval)?
                {
                    if let Some(text) = message_text(&msg) {
                        arg_fields.push(("message".into(), SwiftValue::Str(text)));
                    }
                }
            }
            let mut fields: Vec<(String, SwiftValue)> = arg_fields;
            fields.push((
                MODIFIERS_FIELD.into(),
                SwiftValue::Array(Rc::new(Vec::new())),
            ));
            fields.push((
                CHILDREN_FIELD.into(),
                SwiftValue::Array(Rc::new(content.into_iter().collect())),
            ));
            fields.push((BINDING_FIELD.into(), binding));
            if let Some(d @ SwiftValue::Closure(_)) = rec.get("_onDismiss").cloned() {
                fields.push((
                    HANDLERS_FIELD.into(),
                    SwiftValue::Struct(Rc::new(StructObj {
                        type_name: HANDLERS_TYPE.into(),
                        fields: vec![("dismiss".into(), d)],
                    })),
                ));
            }
            nodes.push(SwiftValue::Struct(Rc::new(StructObj {
                type_name: PRESENTATION_NODE_KIND.into(),
                fields,
            })));
        }
        if nodes.is_empty() {
            return Ok(());
        }
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
        kids.extend(nodes);
        obj.fields[pos].1 = SwiftValue::Array(Rc::new(kids));
        Ok(())
    }

    /// Realize a session-owned `NavigationStack`'s pushed screens from its
    /// per-stack `nav_stack` state (destination-based links and value-based
    /// pushes both live here). Each entry is realized fresh for `@State`
    /// liveness.
    fn realize_session_screens(&mut self, id: &str) -> Result<Vec<SwiftValue>, EvalError> {
        let Some(pushed) = self.nav_stack.get(id).cloned() else {
            return Ok(Vec::new());
        };
        let mut screens = Vec::new();
        for dest in pushed {
            if let Some(screen) = crate::realize_pushed_screen(self.interp, &dest)
                .map_err(crate::std_error_to_eval)?
            {
                screens.push(screen);
            }
        }
        Ok(screens)
    }

    /// Realize a path-bound `NavigationStack`'s screens (ADR-0013 §1): read the
    /// bound path's items, match each to a `.navigationDestination(for:)`
    /// registration in the stack's subtree (by the value's runtime type), and
    /// evaluate that closure with the value. The path binding is the source of
    /// truth, so external mutation (a `Button` appending to `path`) re-renders
    /// to the new depth. Unmatched items produce no screen.
    fn realize_path_screens(
        &mut self,
        stack: &StructObj,
        binding: &SwiftValue,
    ) -> Result<Vec<SwiftValue>, EvalError> {
        let items =
            match crate::read_path_items(self.interp, binding).map_err(crate::std_error_to_eval)? {
                Some(items) => items,
                None => return Ok(Vec::new()),
            };
        // Collect the stack subtree's destination registrations once.
        let destinations = collect_nav_destinations(&SwiftValue::Struct(Rc::new(stack.clone())));
        let mut screens = Vec::new();
        for item in items {
            let ty = item.type_name();
            if let Some(closure) = destinations.get(&ty) {
                let record = crate::pushed_value(SwiftValue::Closure(*closure), item);
                if let Some(screen) = crate::realize_pushed_screen(self.interp, &record)
                    .map_err(crate::std_error_to_eval)?
                {
                    screens.push(screen);
                }
            }
        }
        Ok(screens)
    }

    /// Push a value-based link's value onto its enclosing stack (ADR-0013 §1).
    /// Resolves the nearest `.navigationDestination(for:)` in the stack subtree
    /// matching the value's runtime type; with no match the tap is a no-op. A
    /// path-bound stack appends to the bound path (its source of truth); a
    /// session-owned stack stores the resolved closure + value for fresh
    /// re-realization each render.
    fn push_value_link(
        &mut self,
        tree: &SwiftValue,
        stack_id: &str,
        value: SwiftValue,
    ) -> Result<(), EvalError> {
        let Some(stack) = find_node(tree, stack_id) else {
            return Ok(());
        };
        let SwiftValue::Struct(stack_obj) = stack else {
            return Ok(());
        };
        let ty = value.type_name();
        let destinations = collect_nav_destinations(stack);
        let Some(closure) = destinations.get(&ty).copied() else {
            // Unmatched value → no-op (no matching destination registered).
            return Ok(());
        };
        if let Some(binding) = stack_obj.get(BINDING_FIELD).cloned() {
            crate::path_append(self.interp, &binding, value).map_err(crate::std_error_to_eval)?;
        } else {
            let record = crate::pushed_value(SwiftValue::Closure(closure), value);
            self.nav_stack
                .entry(stack_id.to_string())
                .or_default()
                .push(record);
        }
        Ok(())
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
            // An `AsyncImage` load-phase update (ADR-0013 §4): the host reports
            // `"empty"` / `"success"` / `"failure"` as the image URL loads.
            // The phase is stored keyed by node id; the next render evaluates
            // the content/placeholder/phase closure accordingly. An unknown id
            // or a node that is no longer an AsyncImage is silently ignored.
            "imagePhase" => {
                let phase = match &event.value {
                    Some(SwiftValue::Str(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                };
                if !phase.is_empty() {
                    let url = find_async_image_url(&tree, &event.id).unwrap_or_default();
                    self.image_phase.insert(event.id.clone(), (phase, url));
                }
            }
            // A `NavigationStack` back affordance (ADR-0013 §1): pop the topmost
            // pushed screen off the stack keyed by the event id. A path-bound
            // stack pops the bound path (the source of truth); a session-owned
            // stack pops its per-stack state. A `back` on an empty stack is a
            // no-op.
            "back" => {
                if let Some(binding) = find_binding(&tree, &event.id) {
                    crate::path_remove_last(self.interp, &binding)
                        .map_err(crate::std_error_to_eval)?;
                } else if let Some(stack) = self.nav_stack.get_mut(&event.id) {
                    stack.pop();
                    if stack.is_empty() {
                        self.nav_stack.remove(&event.id);
                    }
                }
            }
            // A presentation dismissal (ADR-0019): the host swiped/tapped-away
            // a `.sheet`/`.popover`/`.fullScreenCover`. Write the gating binding
            // back to closed (`false` for `isPresented:`, `nil` for `item:`) so
            // the next render drops the `Presentation` node, then fire the
            // `onDismiss` closure if one was captured.
            "dismiss" => {
                if let Some(binding) = find_binding(&tree, &event.id) {
                    let current = self.interp.get_member(&binding, "wrappedValue")?;
                    let closed = match current {
                        SwiftValue::Bool(_) => SwiftValue::Bool(false),
                        _ => SwiftValue::Nil,
                    };
                    self.interp.set_member(&binding, "wrappedValue", closed)?;
                }
                if let Some(closure_id) = find_handler(&tree, &event.id, "dismiss") {
                    self.interp.invoke_closure(closure_id, Vec::new())?;
                }
            }
            // Any other event routes to the node's handler map by name. A `tap`
            // on a `NavigationLink` is special-cased first (ADR-0013 §1): a
            // destination-based link captures its destination onto the enclosing
            // stack's pushed state; a value-based link resolves the nearest
            // matching `.navigationDestination(for:)` and pushes that.
            name => {
                if name == "tap" {
                    match find_nav_link(&tree, &event.id) {
                        Some(NavLinkTarget::Destination {
                            stack_id,
                            destination,
                        }) => {
                            self.nav_stack
                                .entry(stack_id)
                                .or_default()
                                .push(destination);
                        }
                        Some(NavLinkTarget::Value { stack_id, value }) => {
                            self.push_value_link(&tree, &stack_id, value)?;
                        }
                        None => {
                            if let Some(closure_id) = find_handler(&tree, &event.id, name) {
                                self.interp.invoke_closure(closure_id, Vec::new())?;
                            }
                        }
                    }
                } else if let Some(closure_id) = find_handler(&tree, &event.id, name) {
                    self.interp
                        .invoke_closure(closure_id, event_arguments(event.value.as_ref()))?;
                }
                // Alert/confirmationDialog actions always dismiss (SwiftUI
                // semantics): tapping any button inside an `alert`/
                // `confirmationDialog` presentation closes it. Write the
                // enclosing presentation's gating binding to closed after the
                // action's own closure has run.
                if name == "tap" {
                    if let Some(binding) = alert_ancestor_binding(&tree, &event.id) {
                        let current = self.interp.get_member(&binding, "wrappedValue")?;
                        let closed = match current {
                            SwiftValue::Bool(_) => SwiftValue::Bool(false),
                            _ => SwiftValue::Nil,
                        };
                        self.interp.set_member(&binding, "wrappedValue", closed)?;
                    }
                }
            }
        }
        // Event closures may spawn `Task {}` work. The session owns the
        // cooperative executor boundary, so drain before rendering/diffing and
        // make state mutations after `await` observable in this response.
        self.interp.drain_pending_tasks()?;
        let tree = self.render()?;
        let tree = self.run_watchers(baseline, tree)?;
        if self.task_lifecycle_started {
            self.run_new_tasks(tree)
        } else {
            Ok(tree)
        }
    }

    /// Fire every newly-mounted or changed-id `.task {}` closure on the current
    /// tree (ADR-0013 §3), then re-render and run `onChange` watchers.
    ///
    /// The host calls this once after a successful mount to run
    /// appear-time async work. The render session owns the executor drain: any
    /// `Task {}` spawned by a lifecycle closure completes before the following
    /// render, so awaited state mutations are visible in its UIIR patches. A
    /// `.task` and a `.onAppear` on the same view coexist — they bind distinct
    /// handler keys and fire independently. With no `.task` modifiers this is a
    /// re-render that emits an empty patch set. Same `Result` shape as
    /// `render`/`dispatch` so the caller can diff before/after identically.
    pub fn run_mount_tasks(&mut self) -> Result<SwiftValue, EvalError> {
        let tree = match &self.current {
            Some(tree) => tree.clone(),
            None => self.render()?,
        };
        self.task_lifecycle_started = true;
        self.run_new_tasks(tree)
    }

    /// Run tasks whose node has just mounted or whose explicit task identity
    /// differs from the last observed one. The caller supplies the pre-task
    /// rendered tree so its `onChange` baseline includes mutations made by the
    /// task body.
    fn run_new_tasks(&mut self, baseline_tree: SwiftValue) -> Result<SwiftValue, EvalError> {
        let baseline = collect_watch_values(&baseline_tree);
        let mut mounted = HashSet::new();
        let mut task_closures = Vec::new();
        // Capture the task identity before executing closures: a closure may
        // synchronously mutate state, but cannot cause itself to run twice in
        // this lifecycle pass.
        crate::tree::walk(&baseline_tree, &mut |id, _, obj| {
            let Some(SwiftValue::Struct(handlers)) = obj.get(HANDLERS_FIELD) else {
                return;
            };
            let Some(SwiftValue::Closure(cid)) = handlers.get("task") else {
                return;
            };
            mounted.insert(id.to_string());
            let identity = task_identity(obj);
            if self.task_ids.get(id) != Some(&identity) {
                self.task_ids.insert(id.to_string(), identity);
                task_closures.push(*cid);
            }
        });
        self.task_ids.retain(|id, _| mounted.contains(id));
        for cid in task_closures {
            self.interp.invoke_closure(cid, Vec::new())?;
        }
        // Session, not host, owns task driving. Hosts only request a lifecycle
        // pass or dispatch an event; this boundary ensures the next tree sees
        // all detached `Task {}` effects spawned by either closure kind.
        self.interp.drain_pending_tasks()?;
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

/// Convert an optional host event payload into closure arguments. A scalar is
/// one argument; an array represents the positional arguments of callbacks
/// such as `onScrollPhaseChange`. No payload preserves the zero-argument
/// action convention used by buttons, gestures, and lifecycle modifiers.
fn event_arguments(value: Option<&SwiftValue>) -> Vec<SwiftValue> {
    match value {
        Some(SwiftValue::Array(values)) => values.iter().cloned().collect(),
        Some(value) => vec![value.clone()],
        None => Vec::new(),
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
    let mut out = Vec::new();
    crate::tree::walk(tree, &mut |id, _, obj| {
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
    });
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
    let SwiftValue::Struct(obj) = crate::tree::find(tree, target)? else {
        return None;
    };
    match obj.get(HANDLERS_FIELD)? {
        SwiftValue::Struct(handlers) => match handlers.get(event) {
            Some(SwiftValue::Closure(cid)) => Some(*cid),
            _ => None,
        },
        _ => None,
    }
}

/// The resolved target of a tapped `NavigationLink` (ADR-0013 §1): either a
/// destination-based link carrying its captured destination, or a value-based
/// link carrying its `value:` payload. Both name the enclosing stack.
pub enum NavLinkTarget {
    /// A `NavigationLink(destination:)` — push the captured destination.
    Destination {
        stack_id: String,
        destination: SwiftValue,
    },
    /// A `NavigationLink(value:)` — resolve the value against the stack's
    /// `.navigationDestination(for:)` registrations, then push.
    Value { stack_id: String, value: SwiftValue },
}

/// Find the tapped `NavigationLink` at structural path `target` and its
/// enclosing `NavigationStack` (ADR-0013 §1), classifying it as destination- or
/// value-based. The nearest ancestor `NavigationStack` id is threaded down the
/// walk.
pub fn find_nav_link(tree: &SwiftValue, target: &str) -> Option<NavLinkTarget> {
    let (node, stack_id) =
        crate::tree::find_with_ancestor(tree, target, |obj| obj.type_name == "NavigationStack")?;
    let SwiftValue::Struct(obj) = node else {
        return None;
    };
    let sid = stack_id?;
    if let Some(dest) = obj.get(NAV_DESTINATION_FIELD) {
        return Some(NavLinkTarget::Destination {
            stack_id: sid,
            destination: dest.clone(),
        });
    }
    if let Some(value) = obj.get(NAV_VALUE_FIELD) {
        return Some(NavLinkTarget::Value {
            stack_id: sid,
            value: value.clone(),
        });
    }
    None
}

/// Find the node at structural path `target` in `tree` (id scheme shared with
/// `uiir`).
pub fn find_node<'a>(tree: &'a SwiftValue, target: &str) -> Option<&'a SwiftValue> {
    crate::tree::find(tree, target)
}

/// Collect every `.navigationDestination(for:)` registration within `subtree`
/// as a type-name → closure-id map (ADR-0013 §1). A shallower (nearer-the-stack)
/// registration wins over a deeper duplicate of the same type, matching
/// SwiftUI's "nearest enclosing" resolution.
pub fn collect_nav_destinations(subtree: &SwiftValue) -> HashMap<String, usize> {
    let mut acc: HashMap<String, (usize, usize)> = HashMap::new();
    crate::tree::walk(subtree, &mut |_, depth, obj| {
        if let Some(SwiftValue::Struct(map)) = obj.get(NAV_DESTINATIONS_FIELD) {
            for (ty, closure) in &map.fields {
                if let SwiftValue::Closure(cid) = closure {
                    let entry = acc.entry(ty.clone()).or_insert((depth, *cid));
                    if depth < entry.0 {
                        *entry = (depth, *cid);
                    }
                }
            }
        }
    });
    acc.into_iter().map(|(k, (_, cid))| (k, cid)).collect()
}

/// The gating `Binding` of the nearest `alert`/`confirmationDialog`
/// `Presentation` node that encloses `target` (ADR-0019). Used to auto-dismiss
/// an alert when any of its action buttons is tapped. Returns `None` when the
/// tapped node is not inside such a presentation.
fn alert_ancestor_binding(tree: &SwiftValue, target: &str) -> Option<SwiftValue> {
    let (_, ancestor) = crate::tree::find_with_ancestor(tree, target, |obj| {
        obj.type_name == PRESENTATION_NODE_KIND
            && matches!(
                obj.get("style"),
                Some(SwiftValue::Str(s)) if s == "alert" || s == "confirmationDialog"
            )
    })?;
    find_binding(tree, &ancestor?)
}

/// Find the `Binding` value stashed on the control node at structural path
/// `target` (the `_binding` field a `Toggle`/input writes through).
pub fn find_binding(tree: &SwiftValue, target: &str) -> Option<SwiftValue> {
    let SwiftValue::Struct(obj) = crate::tree::find(tree, target)? else {
        return None;
    };
    obj.get(BINDING_FIELD).cloned()
}

/// Collect the structural ids of all v1.5 `AsyncImage` nodes (those with
/// closure fields) that appear in `tree`. Used by `prune_image_phase`.
fn collect_async_image_ids(tree: &SwiftValue) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    crate::tree::walk(tree, &mut |id, _, obj| {
        if obj.type_name == "AsyncImage" && crate::has_async_image_closures(obj) {
            out.insert(id.to_string());
        }
    });
    out
}

/// Find the `url` arg string of the `AsyncImage` node at structural path
/// `target`. Returns `None` when the id is not found or the node is not an
/// `AsyncImage` with a `url` arg.
fn find_async_image_url(tree: &SwiftValue, target: &str) -> Option<String> {
    let SwiftValue::Struct(obj) = crate::tree::find(tree, target)? else {
        return None;
    };
    match obj.get("url") {
        Some(SwiftValue::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{install, uiir, PRELUDE};

    fn counter_interp() -> Interpreter<'static> {
        let src = format!(
            "import SwiftUI\n{PRELUDE}\n{}",
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

    fn app_interp() -> Interpreter<'static> {
        let src = format!(
            "import SwiftUI\n{PRELUDE}\n{}",
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

@main struct DemoApp: App {
    var body: some Scene {
        Group {
            WindowGroup { CounterView() }
        }
    }
}
"#
        );
        let analysis = tswift_frontend::Analysis::analyze(&src, "app.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let out: &'static mut std::io::Sink = Box::leak(Box::new(std::io::sink()));
        let mut interp = Interpreter::new(out);
        install(&mut interp);
        interp.run(analysis).expect("run");
        interp
    }

    #[test]
    fn app_session_unwraps_grouped_window_and_preserves_state() {
        let mut interp = app_interp();
        let mut session = Session::new_app(&mut interp, "DemoApp").expect("app session");

        let first = session.render().expect("render");
        assert!(uiir::to_json(&first).contains(r#""verbatim":"0""#));

        let after = session
            .dispatch(&Event {
                id: "0.1".into(),
                event: "tap".into(),
                value: None,
            })
            .expect("dispatch");
        assert!(uiir::to_json(&after).contains(r#""verbatim":"1""#));
    }

    #[test]
    fn app_session_rejects_multiple_top_level_window_groups() {
        let src = format!(
            "import SwiftUI\n{PRELUDE}\n{}",
            r#"
struct A: View {
    var body: some View { Text("A") }
}

struct B: View {
    var body: some View { Text("B") }
}

@main struct DemoApp: App {
    var body: some Scene {
        WindowGroup { A() }
        WindowGroup { B() }
    }
}
"#
        );
        let analysis = tswift_frontend::Analysis::analyze(&src, "app.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let out: &'static mut std::io::Sink = Box::leak(Box::new(std::io::sink()));
        let mut interp = Interpreter::new(out);
        install(&mut interp);
        interp.run(analysis).expect("run");

        let error = match Session::new_app(&mut interp, "DemoApp") {
            Ok(_) => panic!("multiple top-level WindowGroups must be rejected"),
            Err(error) => error,
        };
        assert_eq!(error, EvalError::Unsupported("multiple windows".into()));
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
            "import SwiftUI\n{PRELUDE}\n{}",
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
            "import SwiftUI\n{PRELUDE}\n{}",
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
            "import SwiftUI\n{PRELUDE}\n{}",
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
            "import SwiftUI\n{PRELUDE}\n{}",
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
            "import SwiftUI\n{PRELUDE}\n{}",
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
            "import SwiftUI\n{PRELUDE}\n{}",
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
            "import SwiftUI\n{PRELUDE}\n{}",
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
        let src = format!("import SwiftUI\n{PRELUDE}\n{program}");
        let analysis = tswift_frontend::Analysis::analyze(&src, "t.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let out: &'static mut std::io::Sink = Box::leak(Box::new(std::io::sink()));
        let mut interp = Interpreter::new(out);
        // Mirror the CLI host: stdlib intrinsics (`Array.append`/`removeLast`,
        // needed by `NavigationPath` and typed-array path bindings) plus the
        // SwiftUI view layer.
        tswift_std::install(&mut interp);
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
    fn dispatch_forwards_scalar_payload_to_event_handler_closure() {
        let mut interp = events_interp(
            r#"
struct HoverView: View {
    @State private var status = "idle"
    var body: some View {
        Text(status)
            .onHover { hovering in status = hovering ? "hovered" : "left" }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "HoverView").expect("session");
        session.render().expect("render");
        let hover = Event {
            id: "0".into(),
            event: "hover".into(),
            value: Some(SwiftValue::Bool(true)),
        };
        let after = session.dispatch(&hover).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains("hovered"),
            "event payload reached onHover closure: {}",
            uiir::to_json(&after)
        );
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
    fn task_modifier_fires_on_run_mount_tasks() {
        let mut interp = events_interp(
            r#"
struct TaskView: View {
    @State private var loaded = false
    var body: some View {
        Text(loaded ? "Loaded" : "Loading...")
            .task { loaded = true }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "TaskView").expect("session");
        let initial = session.render().expect("render");
        assert!(
            uiir::to_json(&initial).contains("Loading..."),
            "initial tree shows Loading...: {}",
            uiir::to_json(&initial)
        );
        let after = session.run_mount_tasks().expect("run_mount_tasks");
        assert!(
            uiir::to_json(&after).contains("Loaded"),
            "task fired and updated state: {}",
            uiir::to_json(&after)
        );
    }

    #[test]
    fn task_modifier_coexists_with_on_appear() {
        let mut interp = events_interp(
            r#"
struct TwoView: View {
    @State private var a = false
    @State private var b = false
    var body: some View {
        Text(a && b ? "both" : "neither")
            .onAppear { a = true }
            .task { b = true }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "TwoView").expect("session");
        session.render().expect("render");
        // The host fires `appear` on the root (id "0"); then tasks run.
        let appear = Event {
            id: "0".into(),
            event: "appear".into(),
            value: None,
        };
        session.dispatch(&appear).expect("appear");
        let after = session.run_mount_tasks().expect("run_mount_tasks");
        assert!(
            uiir::to_json(&after).contains("both"),
            "onAppear and task both fired: {}",
            uiir::to_json(&after)
        );
    }

    #[test]
    fn task_modifier_runs_await_inline() {
        // The load-bearing claim for networked `.task`: an `await` inside the
        // task closure runs inline under the cooperative executor, so the
        // re-render after `run_mount_tasks` observes the mutated @State.
        let mut interp = events_interp(
            r#"
struct TaskView: View {
    @State private var label = "Loading..."
    func fetchLabel() async -> String { "Loaded" }
    var body: some View {
        Text(label)
            .task { label = await fetchLabel() }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "TaskView").expect("session");
        let initial = session.render().expect("render");
        assert!(
            uiir::to_json(&initial).contains("Loading..."),
            "initial tree shows Loading...: {}",
            uiir::to_json(&initial)
        );
        let after = session.run_mount_tasks().expect("run_mount_tasks");
        assert!(
            uiir::to_json(&after).contains("Loaded"),
            "await inside .task ran inline and updated state: {}",
            uiir::to_json(&after)
        );
    }

    #[test]
    fn task_modifier_restarts_when_its_id_changes() {
        let mut interp = events_interp(
            r#"
struct TaskIDView: View {
    @State private var id = 1
    @State private var runs = 0
    var body: some View {
        VStack {
            Text("runs=\(runs)")
            Button("Next") { id += 1 }
        }
        .task(id: id) { runs += 1 }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "TaskIDView").expect("session");
        session.render().expect("render");
        let mounted = session.run_mount_tasks().expect("mount task");
        assert!(
            uiir::to_json(&mounted).contains("runs=1"),
            "mounted: {mounted:?}"
        );
        let after = session
            .dispatch(&Event {
                id: "0.1".into(),
                event: "tap".into(),
                value: None,
            })
            .expect("dispatch");
        assert!(uiir::to_json(&after).contains("runs=2"), "after: {after:?}");
    }

    #[test]
    fn dispatch_drains_spawned_task_before_returning_tree() {
        let mut interp = events_interp(
            r#"
struct SpawnedTaskView: View {
    @State private var label = "idle"
    func loaded() async -> String { "loaded" }
    var body: some View {
        VStack {
            Text(label)
            Button("Load") { Task { label = await loaded() } }
        }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "SpawnedTaskView").expect("session");
        session.render().expect("render");
        let after = session
            .dispatch(&Event {
                id: "0.1".into(),
                event: "tap".into(),
                value: None,
            })
            .expect("dispatch");
        assert!(uiir::to_json(&after).contains("loaded"), "after: {after:?}");
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
    fn value_link_resolves_nearest_matching_destination() {
        // A value-based `NavigationLink(value:)` resolves the enclosing stack's
        // `.navigationDestination(for: Int.self)` and pushes the screen the
        // closure builds from the value.
        let mut interp = events_interp(
            r#"
struct RootView: View {
    var body: some View {
        NavigationStack {
            VStack {
                NavigationLink("Go", value: 42)
            }
            .navigationDestination(for: Int.self) { n in
                Text("Number: \(n)")
            }
        }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "RootView").expect("session");
        let first = session.render().expect("render");
        let json = uiir::to_json(&first);
        assert!(
            !json.contains("_navValue"),
            "value never serialized: {json}"
        );
        assert!(
            !json.contains("Number:"),
            "destination not realized before tap: {json}"
        );
        // Tap the value link (stack 0 > VStack 0.0 > link 0.0.0).
        let tap = Event {
            id: "0.0.0".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&tap).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains("Number: 42"),
            "value link pushes matched destination: {}",
            uiir::to_json(&after)
        );
        // `back` pops it.
        let back = Event {
            id: "0".into(),
            event: "back".into(),
            value: None,
        };
        let after = session.dispatch(&back).expect("dispatch");
        assert!(
            !uiir::to_json(&after).contains("Number: 42"),
            "back pops the value screen: {}",
            uiir::to_json(&after)
        );
    }

    #[test]
    fn value_link_with_no_matching_destination_is_noop() {
        // A value whose type has no registered destination pushes nothing.
        let mut interp = events_interp(
            r#"
struct RootView: View {
    var body: some View {
        NavigationStack {
            VStack {
                NavigationLink("Go", value: "hi")
            }
            .navigationDestination(for: Int.self) { n in
                Text("Number: \(n)")
            }
        }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "RootView").expect("session");
        session.render().expect("render");
        let tap = Event {
            id: "0.0.0".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&tap).expect("dispatch");
        // Only the root VStack child remains; nothing pushed.
        let json = uiir::to_json(&after);
        assert!(
            !json.contains("Number:"),
            "unmatched String value is a no-op: {json}"
        );
    }

    #[test]
    fn path_binding_reflects_push_pop_and_programmatic_append() {
        // A `NavigationStack(path:)` bound to a `NavigationPath`: a value link
        // appends to the path; a `Button` appending to the path pushes too;
        // `back` pops the path. The bound path is the source of truth.
        let mut interp = events_interp(
            r#"
struct RootView: View {
    @State private var path = NavigationPath()
    var body: some View {
        NavigationStack(path: $path) {
            VStack {
                NavigationLink("Go", value: 7)
                Button("Push99") { path.append(99) }
            }
            .navigationDestination(for: Int.self) { n in
                Text("N: \(n)")
            }
        }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "RootView").expect("session");
        let first = session.render().expect("render");
        assert!(
            !uiir::to_json(&first).contains("N: "),
            "empty path pushes nothing: {}",
            uiir::to_json(&first)
        );
        // Tap the value link → path.append(7).
        let tap = Event {
            id: "0.0.0".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&tap).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains("N: 7"),
            "value link appends to bound path: {}",
            uiir::to_json(&after)
        );
        // Programmatic append via the Button (id 0.0.1) pushes a second screen.
        let push = Event {
            id: "0.0.1".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&push).expect("dispatch");
        let json = uiir::to_json(&after);
        assert!(
            json.contains("N: 7") && json.contains("N: 99"),
            "programmatic path.append pushes: {json}"
        );
        // `back` pops the last path item (99), leaving 7.
        let back = Event {
            id: "0".into(),
            event: "back".into(),
            value: None,
        };
        let after = session.dispatch(&back).expect("dispatch");
        let json = uiir::to_json(&after);
        assert!(
            json.contains("N: 7") && !json.contains("N: 99"),
            "back pops the bound path: {json}"
        );
    }

    #[test]
    fn path_binding_accepts_typed_array() {
        // A typed-array binding (`path: $ints`) works too (v1 array form): each
        // element is matched to the `Int` destination.
        let mut interp = events_interp(
            r#"
struct RootView: View {
    @State private var ints: [Int] = []
    var body: some View {
        NavigationStack(path: $ints) {
            VStack {
                Button("Push5") { ints.append(5) }
            }
            .navigationDestination(for: Int.self) { n in
                Text("Row \(n)")
            }
        }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "RootView").expect("session");
        session.render().expect("render");
        let push = Event {
            id: "0.0.0".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&push).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains("Row 5"),
            "typed-array path append pushes: {}",
            uiir::to_json(&after)
        );
    }

    // ── AsyncImage (ADR-0013 §4) ──────────────────────────────────────────────

    fn async_image_interp(extra: &str) -> Interpreter<'static> {
        // AsyncImage requires tswift-foundation's URL type; mirror the CLI
        // host that installs both foundation and SwiftUI layers.
        events_interp_with_foundation(extra)
    }

    fn events_interp_with_foundation(program: &str) -> Interpreter<'static> {
        // Strict import-gating: AsyncImage snippets use Foundation's URL.
        let src = format!("import SwiftUI\nimport Foundation\n{PRELUDE}\n{program}");
        let analysis = tswift_frontend::Analysis::analyze(&src, "t.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let out: &'static mut std::io::Sink = Box::leak(Box::new(std::io::sink()));
        let mut interp = Interpreter::new(out);
        tswift_std::install(&mut interp);
        tswift_foundation::install(&mut interp);
        install(&mut interp);
        interp.run(analysis).expect("run");
        interp
    }

    #[test]
    fn async_image_bare_v1_serializes_url() {
        // A bare `AsyncImage(url:)` serializes the URL string as a `url` arg
        // with no `phase` arg and no children — the host loads natively.
        let mut interp = async_image_interp(
            r#"
struct AView: View {
    var body: some View {
        AsyncImage(url: URL(string: "https://example.com/img.jpg"))
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "AView").expect("session");
        let first = session.render().expect("render");
        let json = uiir::to_json(&first);
        assert!(
            json.contains(r#""kind":"AsyncImage""#),
            "AsyncImage node: {json}"
        );
        assert!(
            json.contains(r#""url":"https://example.com/img.jpg""#),
            "url serialized: {json}"
        );
        assert!(
            !json.contains(r#""phase""#),
            "v1 bare has no phase arg: {json}"
        );
    }

    #[test]
    fn async_image_phase_event_swaps_placeholder_to_content() {
        // `imagePhase` success: session swaps the placeholder for the content
        // closure’s result. Initial render shows ProgressView (placeholder);
        // after success, shows the content image.
        // Note: uses labeled-closure form (`content:`, `placeholder:`) because
        // multi-trailing-closure syntax is not yet supported by the frontend.
        let mut interp = async_image_interp(
            r#"
struct AView: View {
    var body: some View {
        AsyncImage(
            url: URL(string: "https://example.com/img.jpg"),
            content: { image in image },
            placeholder: { Text("loading") }
        )
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "AView").expect("session");
        let first = session.render().expect("render");
        let json = uiir::to_json(&first);
        // Initial render: phase=empty, child is placeholder.
        assert!(
            json.contains(r#""phase":"empty""#),
            "initial phase is empty: {json}"
        );
        assert!(
            json.contains(r#""verbatim":"loading""#),
            "placeholder shown initially: {json}"
        );

        // Dispatch imagePhase success.
        let event = Event {
            id: "0".into(),
            event: "imagePhase".into(),
            value: Some(SwiftValue::Str("success".into())),
        };
        let after = session.dispatch(&event).expect("dispatch");
        let json = uiir::to_json(&after);
        // After success: phase=success, child is content(Image(url)).
        assert!(
            json.contains(r#""phase":"success""#),
            "phase becomes success: {json}"
        );
        assert!(
            json.contains(r#""kind":"Image""#),
            "content image shown on success: {json}"
        );
        assert!(
            json.contains(r#""url":"https://example.com/img.jpg""#),
            "content image carries the URL: {json}"
        );
        assert!(
            !json.contains(r#""verbatim":"loading""#),
            "placeholder gone after success: {json}"
        );
    }

    #[test]
    fn async_image_unknown_id_is_noop() {
        // An imagePhase event on an id that doesn’t exist in the tree is a
        // no-op: the session stores nothing and re-renders cleanly.
        let mut interp = async_image_interp(
            r#"
struct AView: View {
    var body: some View {
        AsyncImage(
            url: URL(string: "https://example.com/img.jpg"),
            content: { image in image },
            placeholder: { Text("loading") }
        )
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "AView").expect("session");
        session.render().expect("render");

        let event = Event {
            id: "99.99".into(),
            event: "imagePhase".into(),
            value: Some(SwiftValue::Str("success".into())),
        };
        let after = session.dispatch(&event).expect("dispatch stays Ok");
        let json = uiir::to_json(&after);
        // Still shows placeholder; unknown id is ignored.
        assert!(
            json.contains(r#""phase":"empty""#),
            "phase unchanged for unknown id: {json}"
        );
    }

    #[test]
    fn async_image_url_change_resets_phase() {
        // When the URL arg changes between renders the stored phase is cleared
        // so the host re-fires load events for the new URL.
        let mut interp = async_image_interp(
            r#"
struct AView: View {
    @State var which = true
    var body: some View {
        VStack {
            Button("swap") { which = !which }
            AsyncImage(
                url: URL(string: which ? "https://a.com/1.jpg" : "https://b.com/2.jpg"),
                content: { image in image },
                placeholder: { Text("loading") }
            )
        }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "AView").expect("session");
        session.render().expect("render");

        // Fire success for the first URL (node id 0.1).
        let success1 = Event {
            id: "0.1".into(),
            event: "imagePhase".into(),
            value: Some(SwiftValue::Str("success".into())),
        };
        let after = session.dispatch(&success1).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains(r#""phase":"success""#),
            "phase is success: {}",
            uiir::to_json(&after)
        );

        // Tap button to swap URL (id 0.0).
        let tap = Event {
            id: "0.0".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&tap).expect("dispatch");
        let json = uiir::to_json(&after);
        // URL changed → phase reset to empty.
        assert!(
            json.contains(r#""phase":"empty""#),
            "URL change resets phase: {json}"
        );
        assert!(
            json.contains("https://b.com/2.jpg"),
            "new URL present: {json}"
        );
    }

    #[test]
    fn async_image_phase_closure_form_renders_success_and_failure() {
        // Phase-closure form: a single trailing closure receives an
        // `AsyncImagePhase` struct; `isSuccess`/`isFailure` control branches.
        let mut interp = async_image_interp(
            r#"
struct AView: View {
    var body: some View {
        AsyncImage(url: URL(string: "https://example.com/img.jpg")) { phase in
            if phase.isSuccess {
                Text("loaded")
            } else if phase.isFailure {
                Text("error")
            } else {
                Text("loading")
            }
        }
    }
}
"#,
        );
        let mut session = Session::new(&mut interp, "AView").expect("session");
        let first = session.render().expect("render");
        let json = uiir::to_json(&first);
        assert!(
            json.contains(r#""phase":"empty""#),
            "initial phase empty: {json}"
        );
        assert!(
            json.contains(r#""verbatim":"loading""#),
            "empty phase shows loading branch: {json}"
        );

        // Success phase.
        let success = Event {
            id: "0".into(),
            event: "imagePhase".into(),
            value: Some(SwiftValue::Str("success".into())),
        };
        let after = session.dispatch(&success).expect("dispatch");
        let json = uiir::to_json(&after);
        assert!(
            json.contains(r#""phase":"success""#),
            "phase becomes success: {json}"
        );
        assert!(
            json.contains(r#""verbatim":"loaded""#),
            "success shows loaded branch: {json}"
        );

        // Failure phase.
        let failure = Event {
            id: "0".into(),
            event: "imagePhase".into(),
            value: Some(SwiftValue::Str("failure".into())),
        };
        let after = session.dispatch(&failure).expect("dispatch");
        let json = uiir::to_json(&after);
        assert!(
            json.contains(r#""phase":"failure""#),
            "phase becomes failure: {json}"
        );
        assert!(
            json.contains(r#""verbatim":"error""#),
            "failure shows error branch: {json}"
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

    fn with_animation_interp() -> Interpreter<'static> {
        let src = format!(
            "import SwiftUI\n{PRELUDE}\n{}",
            r#"
struct FlagView: View {
    @State var flag = false
    var body: some View {
        VStack {
            Text(flag ? "on" : "off")
            Button("Toggle") { withAnimation(.easeInOut(duration: 0.3)) { flag = !flag } }
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
    fn with_animation_button_tap_mutates_state() {
        // Slice 4: `withAnimation(.easeInOut(...)) { flag.toggle() }` inside a
        // Button action must execute the body immediately so the @State changes
        // and the next render reflects the new value.
        let mut interp = with_animation_interp();
        let mut session = Session::new(&mut interp, "FlagView").expect("session");

        let first = session.render().expect("render");
        assert!(
            uiir::to_json(&first).contains(r#""verbatim":"off""#),
            "initial state is off: {}",
            uiir::to_json(&first)
        );

        // Button is the second child of VStack: structural id "0.1".
        let tap = Event {
            id: "0.1".into(),
            event: "tap".into(),
            value: None,
        };
        let after = session.dispatch(&tap).expect("dispatch");
        assert!(
            uiir::to_json(&after).contains(r#""verbatim":"on""#),
            "withAnimation tap must flip flag to true: {}",
            uiir::to_json(&after)
        );
    }
}
