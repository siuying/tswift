use std::cell::RefCell;
use std::rc::Rc;
use std::rc::Rc as StdRc;

use tswift_frontend::{Node, NodeKind};

use crate::env::BindError;
use crate::ops;
use crate::stdlib::{collection_range_bounds, BuiltinReceiver};
use crate::value::{ClassObj, IntValue, IntWidth, SwiftValue};

use super::{
    clone_params, subscript_index, trap, CallArg, ClosureDef, Eval, EvalError, Interpreter, Place,
    Signal,
};

impl<'w> Interpreter<'w> {
    /// Read a computed property off an enum value, if declared.
    pub(super) fn read_enum_computed(
        &mut self,
        value: &SwiftValue,
        name: &str,
    ) -> Result<Option<SwiftValue>, Signal> {
        let SwiftValue::Enum(e) = value else {
            return Ok(None);
        };
        let getter = self
            .enums
            .get(&e.type_name)
            .and_then(|d| d.computed.get(name))
            .filter(|c| !c.is_static)
            .and_then(|c| c.getter);
        match getter {
            Some(body) => self
                .run_with_self(value.clone(), |me| me.eval(&body))
                .map(|(v, _)| Some(v)),
            None => Ok(None),
        }
    }

    /// Read a member off a class instance: a stored field (upgrading weak
    /// references), or a computed getter.
    pub(super) fn read_object_member(&mut self, value: &SwiftValue, name: &str) -> Eval {
        let SwiftValue::Object(obj) = value else {
            return Err(EvalError::Type(format!("`{name}` is not a member")).into());
        };
        let class_name = obj.borrow().class_name.clone();
        if let Some(field) = obj.borrow().get(name).cloned() {
            return Ok(match field {
                SwiftValue::Weak(w) => w
                    .upgrade()
                    .map(SwiftValue::Object)
                    .unwrap_or(SwiftValue::Nil),
                v => v,
            });
        }
        // Computed getter somewhere in the chain.
        let getter = self.class_computed_getter(&class_name, name);
        if let Some(body) = getter {
            self.class_ctx.push(class_name);
            let r = self
                .run_with_self(value.clone(), |me| me.eval(&body))
                .map(|(v, _)| v);
            self.class_ctx.pop();
            return r;
        }
        Err(EvalError::Type(format!("{class_name} has no member `{name}`")).into())
    }

    /// Find a computed getter for `name` walking up the class chain.
    pub(super) fn class_computed_getter(
        &self,
        class_name: &str,
        name: &str,
    ) -> Option<Node<'static>> {
        let mut current = Some(class_name.to_string());
        while let Some(cls) = current {
            let def = self.classes.get(&cls)?;
            if let Some(c) = def.computed.get(name).filter(|c| !c.is_static) {
                return c.getter;
            }
            current = def.superclass.clone();
        }
        None
    }

    /// Set a stored field on a class instance in place, downgrading values
    /// assigned to `weak` fields.
    fn set_object_field(&mut self, obj: &StdRc<RefCell<ClassObj>>, name: &str, value: SwiftValue) {
        let class_name = obj.borrow().class_name.clone();
        let is_weak = self.field_is_weak(&class_name, name);
        let stored = if is_weak {
            match value {
                SwiftValue::Object(o) => SwiftValue::Weak(StdRc::downgrade(&o)),
                other => other,
            }
        } else {
            value
        };
        obj.borrow_mut().set(name, stored);
    }

    /// Whether `name` is a `weak` field anywhere in the class chain.
    fn field_is_weak(&self, class_name: &str, name: &str) -> bool {
        let mut current = Some(class_name.to_string());
        while let Some(cls) = current {
            let Some(def) = self.classes.get(&cls) else {
                break;
            };
            if def.weak_fields.iter().any(|f| f == name) {
                return true;
            }
            current = def.superclass.clone();
        }
        false
    }

    /// Whether a class (or any ancestor) declares a stored/computed member.
    pub(super) fn class_has_member(&self, class_name: &str, name: &str) -> bool {
        let mut current = Some(class_name.to_string());
        while let Some(cls) = current {
            let Some(def) = self.classes.get(&cls) else {
                break;
            };
            if def.stored.iter().any(|p| p.name == name) || def.computed.contains_key(name) {
                return true;
            }
            current = def.superclass.clone();
        }
        false
    }

    /// A subscript read `base[index]` over arrays, strings, or a user
    /// `subscript` getter.
    pub(super) fn eval_subscript(&mut self, node: &Node<'static>) -> Eval {
        let mut kids = node.children();
        let base = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("subscript without a base".into()))?;
        // `Type[index]`: a `static subscript` addressed through the type name.
        if base.kind() == NodeKind::IdentExpr {
            if let Some(type_name) = base.text() {
                let has_static = self
                    .structs
                    .get(&type_name)
                    .is_some_and(|d| d.static_subscript.is_some())
                    || self
                        .classes
                        .get(&type_name)
                        .is_some_and(|d| d.static_subscript.is_some());
                if self.env.get(&type_name).is_none() && has_static {
                    let indices: Vec<SwiftValue> =
                        kids.map(|n| self.eval(&n)).collect::<Result<_, _>>()?;
                    return self.read_static_subscript(&type_name, &indices);
                }
            }
        }
        let base_value = self.eval(&base)?;
        let index_nodes: Vec<Node<'static>> = kids.collect();
        // A single one-sided range index (`a[2...]`, `a[..<2]`, `a[...2]`) is
        // resolved against the base collection's length into a concrete
        // `Range` before the generic index evaluation, which has no notion of
        // partial ranges.
        if let [only] = index_nodes.as_slice() {
            if let Some(range) = self.eval_partial_range_index(only, &base_value)? {
                return self.read_subscript(&base_value, &[range]);
            }
        }
        let indices: Vec<SwiftValue> = index_nodes
            .iter()
            .map(|n| self.eval(n))
            .collect::<Result<_, _>>()?;
        self.read_subscript(&base_value, &indices)
    }

    /// If `node` is a one-sided range form (`..<n` / `...n` prefix or `n...`
    /// postfix), resolve it to a concrete `Range` over `base`'s length; else
    /// `None`. The lower bound of an up-to/through range is `0`; the upper
    /// bound of a from range is the collection's element count.
    fn eval_partial_range_index(
        &mut self,
        node: &Node<'static>,
        base: &SwiftValue,
    ) -> Result<Option<SwiftValue>, Signal> {
        let len = match base {
            SwiftValue::Array(items) => items.len() as i128,
            SwiftValue::Str(s) => crate::graphemes(s).len() as i128,
            _ => return Ok(None),
        };
        let op = node.op_text();
        let bound_int = |this: &mut Self, n: &Node<'static>| -> Result<i128, Signal> {
            match this.eval(n)? {
                SwiftValue::Int(i) => Ok(i.raw),
                other => Err(EvalError::Type(format!(
                    "range bound must be an integer, found {}",
                    other.type_name()
                ))
                .into()),
            }
        };
        let child = node.first_child();
        match (node.kind(), op.as_deref()) {
            (NodeKind::PrefixExpr, Some("..<")) => {
                let hi = bound_int(self, &child.unwrap())?;
                Ok(Some(SwiftValue::Range {
                    lo: 0,
                    hi,
                    inclusive: false,
                }))
            }
            (NodeKind::PrefixExpr, Some("...")) => {
                let hi = bound_int(self, &child.unwrap())?;
                Ok(Some(SwiftValue::Range {
                    lo: 0,
                    hi,
                    inclusive: true,
                }))
            }
            (NodeKind::PostfixExpr, Some("...")) => {
                let lo = bound_int(self, &child.unwrap())?;
                Ok(Some(SwiftValue::Range {
                    lo,
                    hi: len,
                    inclusive: false,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Evaluate a `static subscript` declared on `type_name`, addressed as
    /// `Type[index]`. No `self` is bound; only the index parameters are.
    fn read_static_subscript(&mut self, type_name: &str, indices: &[SwiftValue]) -> Eval {
        let (params, body) = {
            let m = self
                .structs
                .get(type_name)
                .and_then(|d| d.static_subscript.as_ref())
                .or_else(|| {
                    self.classes
                        .get(type_name)
                        .and_then(|d| d.static_subscript.as_ref())
                })
                .expect("static subscript exists");
            (clone_params(&m.params), m.body)
        };
        let args: Vec<CallArg> = indices
            .iter()
            .map(|v| CallArg {
                label: None,
                value: v.clone(),
                place: None,
            })
            .collect();
        self.env.push();
        let bound = self.bind_params(&params, args);
        let result = match bound {
            Ok(_) => match body {
                Some(b) => self.eval(&b),
                None => Ok(SwiftValue::Void),
            },
            Err(e) => Err(e),
        };
        self.env.pop();
        match result {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    /// Assign `base[index] = value` (compound ops supported) over arrays,
    /// dictionaries, and user `subscript { set }`s. A nested subscript base
    /// (`m[i][j] = v`) is handled by read-modify-write through `base`.
    fn assign_subscript(&mut self, target: &Node<'static>, rhs: &Node<'static>, op: &str) -> Eval {
        let mut kids = target.children();
        let base = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("subscript without a base".into()))?;
        let index_nodes: Vec<Node<'static>> = kids.collect();
        if index_nodes.is_empty() {
            return Err(EvalError::Unsupported("subscript without an index".into()).into());
        }
        let index_values: Vec<SwiftValue> = index_nodes
            .iter()
            .map(|n| self.eval(n))
            .collect::<Result<_, _>>()?;
        let current = self.eval(&base)?;

        // The leaf value to store: for a compound op, fold against the element
        // currently at that index.
        let new_value = if op == "=" {
            self.eval(rhs)?
        } else {
            let cur_elem = self.read_subscript(&current, &index_values)?;
            let r = self.eval(rhs)?;
            ops::binary(op.trim_end_matches('='), &cur_elem, &r).map_err(trap)?
        };

        // Remember the original class identity (if any) so an in-place mutation
        // can be told apart from a whole-value replacement below.
        let prev_ref = match &current {
            SwiftValue::Object(o) => Some(o.clone()),
            _ => None,
        };
        let updated = self.set_subscript_element(current, &index_values, new_value)?;
        // A class instance is a reference: when the write mutated *the same*
        // instance in place, there is nothing to rebind (and re-assigning to a
        // `let` binding would be illegal). A whole-value replacement
        // (`obj[keyPath: \.self] = other`) yields a different instance and still
        // writes back.
        if let (Some(prev), SwiftValue::Object(now)) = (&prev_ref, &updated) {
            if StdRc::ptr_eq(prev, now) {
                return Ok(SwiftValue::Void);
            }
        }
        self.assign_value_to(&base, updated)
    }

    /// Return a copy of `container` with `container[indices]` set to `value`,
    /// dispatching over arrays, dictionaries, and user struct subscript setters.
    fn set_subscript_element(
        &mut self,
        container: SwiftValue,
        indices: &[SwiftValue],
        value: SwiftValue,
    ) -> Eval {
        debug_assert!(
            !indices.is_empty(),
            "set_subscript_element requires at least one index"
        );
        // `container[keyPath: kp] = value` — write through a (writable) key path.
        if let [idx] = indices {
            if let Some(components) = self.keypath_components(idx) {
                return self.set_keypath(container, &components, value);
            }
        }
        // A user `subscript { set }` on a struct runs the setter with `self`
        // mutable, the index parameters, and the `newValue` binding.
        if let SwiftValue::Struct(obj) = &container {
            let type_name = obj.type_name.clone();
            let selected = self.structs.get(&type_name).and_then(|d| {
                d.subscripts
                    .iter()
                    .find(|s| s.params.len() == indices.len())
                    .map(|s| (clone_params(&s.params), s.setter, s.setter_param.clone()))
            });
            if let Some((params, setter, setter_param)) = selected {
                let setter_body = setter.ok_or_else(|| {
                    EvalError::Type(format!("{type_name} subscript is read-only"))
                })?;
                let args: Vec<CallArg> = indices
                    .iter()
                    .map(|v| CallArg {
                        label: None,
                        value: v.clone(),
                        place: None,
                    })
                    .collect();
                let saved_env = self.env.enter_isolated();
                self.env.declare("self", container.clone(), true);
                let bound = self.bind_params(&params, args);
                let outcome = match bound {
                    Ok(_) => {
                        self.env.declare(&setter_param, value, false);
                        self.eval(&setter_body)
                    }
                    Err(e) => Err(e),
                };
                let updated_self = self.env.get("self").unwrap_or_else(|| container.clone());
                self.env.restore(saved_env);
                match outcome {
                    Ok(_) | Err(Signal::Return(_)) => {}
                    Err(e) => return Err(e),
                }
                return Ok(updated_self);
            }
            return Err(EvalError::Type(format!(
                "{type_name} has no subscript taking {} index argument(s)",
                indices.len()
            ))
            .into());
        }

        let index_value = indices
            .first()
            .cloned()
            .expect("at least one index checked by caller");
        // `dict[key] = value` inserts/updates; `dict[key] = nil` removes.
        // When `indices.len() > 1` (e.g. `dict[k, default:]`), only
        // `indices[0]` is the key; the compound-op read already folded the
        // `default:` in via `read_subscript`, so extra indices are ignored here.
        if let SwiftValue::Dict(pairs) = &container {
            let mut new_pairs = pairs.as_ref().clone();
            let existing = new_pairs.iter().position(|(k, _)| *k == index_value);
            match (existing, matches!(value, SwiftValue::Nil)) {
                (Some(i), true) => {
                    new_pairs.remove(i);
                }
                (Some(i), false) => new_pairs[i].1 = value,
                (None, true) => {}
                (None, false) => new_pairs.push((index_value, value)),
            }
            return Ok(SwiftValue::Dict(StdRc::new(new_pairs)));
        }
        let idx = subscript_index(&[index_value])?;
        let SwiftValue::Array(items) = &container else {
            return Err(EvalError::Type("subscript assignment requires an array".into()).into());
        };
        if idx >= items.len() {
            return Err(trap(format!("index {idx} out of range")));
        }
        let mut new_items = items.as_ref().clone();
        new_items[idx] = value;
        Ok(SwiftValue::Array(StdRc::new(new_items)))
    }

    /// Write `value` back to the storage named by an lvalue `node`. A subscript
    /// lvalue recurses (so `m[i][j] = v` updates the inner container, then
    /// stores it back into the outer one); recursion terminates when the base is
    /// a variable/member rather than another subscript, which resolves to a
    /// place.
    fn assign_value_to(&mut self, node: &Node<'static>, value: SwiftValue) -> Eval {
        if node.kind() == NodeKind::SubscriptExpr {
            let mut kids = node.children();
            let inner_base = kids
                .next()
                .ok_or_else(|| EvalError::Unsupported("subscript without a base".into()))?;
            let idx_values: Vec<SwiftValue> = kids
                .collect::<Vec<_>>()
                .iter()
                .map(|n| self.eval(n))
                .collect::<Result<_, _>>()?;
            let container = self.eval(&inner_base)?;
            let updated = self.set_subscript_element(container, &idx_values, value)?;
            return self.assign_value_to(&inner_base, updated);
        }
        let place = self
            .resolve_place(node)
            .ok_or_else(|| EvalError::Unsupported("subscript target is not assignable".into()))?;
        self.write_place(&place, value)?;
        Ok(SwiftValue::Void)
    }

    /// Read `base[indices]`.
    fn read_subscript(&mut self, base: &SwiftValue, indices: &[SwiftValue]) -> Eval {
        // `base[keyPath: kp]` — a key-path subscript walks the path from `base`.
        if let [idx] = indices {
            if let Some(components) = self.keypath_components(idx) {
                return self.apply_keypath(base.clone(), &components);
            }
        }
        // `base[range]` — slice an array or string by an integer range
        // (two-sided `a..<b`/`a...b` or a one-sided partial range resolved
        // by `eval_subscript` against the collection length).
        if let [SwiftValue::Range { lo, hi, inclusive }] = indices {
            let (lo, hi, inclusive) = (*lo, *hi, *inclusive);
            match base {
                SwiftValue::Array(items) => {
                    let range = SwiftValue::Range { lo, hi, inclusive };
                    let (start, end) = collection_range_bounds(&range, items.len(), "subscript")?;
                    return Ok(SwiftValue::Array(Rc::new(items[start..end].to_vec())));
                }
                SwiftValue::Str(s) => {
                    let chars = crate::graphemes(s);
                    let range = SwiftValue::Range { lo, hi, inclusive };
                    let (start, end) = collection_range_bounds(&range, chars.len(), "subscript")?;
                    return Ok(SwiftValue::Str(chars[start..end].concat()));
                }
                _ => {}
            }
        }
        match base {
            SwiftValue::Array(items) => {
                let i = subscript_index(indices)?;
                items
                    .get(i)
                    .cloned()
                    .ok_or_else(|| trap(format!("index {i} out of range")))
            }
            // `dict[key]` → the value, or `nil` when absent. `dict[key, default:]`
            // returns the default instead of `nil` when the key is missing.
            SwiftValue::Dict(pairs) => {
                let key = indices
                    .first()
                    .ok_or_else(|| EvalError::Type("dictionary subscript needs a key".into()))?;
                Ok(pairs
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| indices.get(1).cloned().unwrap_or(SwiftValue::Nil)))
            }
            SwiftValue::Str(s) => {
                let i = subscript_index(indices)?;
                // Index by extended grapheme cluster (Swift `Character`), so
                // string indexing agrees with `count` and iteration.
                crate::graphemes(s)
                    .into_iter()
                    .nth(i)
                    .map(SwiftValue::Str)
                    .ok_or_else(|| trap(format!("string index {i} out of range")))
            }
            SwiftValue::Struct(obj) => {
                let type_name = obj.type_name.clone();
                // `IndexPath[i]` reads its `i`th element (a Foundation builtin
                // backed by a `_indexes` array, with no user `subscript`).
                if type_name == "IndexPath" {
                    if let Some(SwiftValue::Array(items)) = obj.get("_indexes") {
                        let i = subscript_index(indices)?;
                        return items
                            .get(i)
                            .cloned()
                            .ok_or_else(|| trap(format!("index {i} out of range")));
                    }
                }
                // Select the overload whose arity matches the index count.
                let getter = self.structs.get(&type_name).and_then(|d| {
                    d.subscripts
                        .iter()
                        .find(|s| s.params.len() == indices.len())
                        .map(|s| (clone_params(&s.params), s.getter))
                });
                if let Some((params, body)) = getter {
                    let args: Vec<CallArg> = indices
                        .iter()
                        .map(|v| CallArg {
                            label: None,
                            value: v.clone(),
                            place: None,
                        })
                        .collect();
                    let saved_env = self.env.enter_isolated();
                    self.env.declare("self", base.clone(), false);
                    let bound = self.bind_params(&params, args);
                    let result = match bound {
                        Ok(_) => match body {
                            Some(b) => self.eval(&b),
                            None => Ok(SwiftValue::Void),
                        },
                        Err(e) => Err(e),
                    };
                    self.env.restore(saved_env);
                    return match result {
                        Ok(v) => Ok(v),
                        Err(Signal::Return(v)) => Ok(v),
                        Err(e) => Err(e),
                    };
                }
                Err(EvalError::Type(format!("{type_name} has no subscript")).into())
            }
            other => Err(EvalError::Type(format!("cannot subscript {}", other.type_name())).into()),
        }
    }

    /// The default initializer of a lazy stored property, if `name` names one.
    fn lazy_default(&self, type_name: &str, name: &str) -> Option<Node<'static>> {
        self.structs.get(type_name).and_then(|d| {
            d.stored
                .iter()
                .find(|p| p.name == name && p.lazy)
                .and_then(|p| p.default)
        })
    }

    /// Whether a struct type declares a stored/computed property or method.
    pub(super) fn struct_has_member(&self, type_name: &str, name: &str) -> bool {
        self.structs.get(type_name).is_some_and(|d| {
            d.computed.contains_key(name)
                || d.methods.contains_key(name)
                || d.stored.iter().any(|p| p.name == name)
        })
    }

    /// Read a property off a struct value: a stored field, or a computed
    /// getter run with `self` bound.
    pub(super) fn read_struct_member(&mut self, value: &SwiftValue, name: &str) -> Eval {
        let SwiftValue::Struct(obj) = value else {
            return Err(EvalError::Type(format!(
                "`{name}` is not a member of {}",
                value.type_name()
            ))
            .into());
        };
        // Projected value `$name` reads the wrapper's `projectedValue`.
        if let Some(stripped) = name.strip_prefix('$') {
            if self.wrapped_field(&obj.type_name, stripped) {
                if let Some(wrapper) = obj.get(stripped).cloned() {
                    return self.read_struct_member(&wrapper, "projectedValue");
                }
            }
        }
        if let Some(v) = obj.get(name) {
            // A wrapped stored property exposes its wrapper's `wrappedValue`.
            if self.wrapped_field(&obj.type_name, name) {
                return self.read_struct_member(&v.clone(), "wrappedValue");
            }
            return Ok(v.clone());
        }
        let getter = self
            .structs
            .get(&obj.type_name)
            .and_then(|d| d.computed.get(name))
            .filter(|c| !c.is_static)
            .and_then(|c| c.getter)
            .or_else(|| self.protocol_default_getter(&obj.type_name, name));
        if let Some(body) = getter {
            return self
                .run_with_self(value.clone(), |me| me.eval(&body))
                .map(|(v, _)| v);
        }
        // `@dynamicMemberLookup`: an unresolved member name routes through a
        // `subscript(dynamicMember:)` getter, passing the name as a string.
        if let Some(v) = self.dynamic_member_read(value, &obj.type_name, name)? {
            return Ok(v);
        }
        Err(EvalError::Type(format!("struct {} has no member `{name}`", obj.type_name)).into())
    }

    /// Find a `subscript(dynamicMember:)` getter on `type_name` and invoke it
    /// with `member` as a string key. Returns `None` when the type declares no
    /// dynamic-member subscript, so the caller can fall through to its error.
    fn dynamic_member_read(
        &mut self,
        receiver: &SwiftValue,
        type_name: &str,
        member: &str,
    ) -> Result<Option<SwiftValue>, Signal> {
        let getter = self.structs.get(type_name).and_then(|d| {
            if !d.dynamic_member_lookup {
                return None;
            }
            // The dynamic-member subscript is the single `String`-keyed overload
            // (its `dynamicMember` argument label is not retained in the AST, so
            // the `@dynamicMemberLookup` attribute plus a one-`String`-parameter
            // signature identifies it — and disambiguates it from an ordinary
            // single-parameter subscript such as `subscript(_ i: Int)`).
            d.subscripts
                .iter()
                .find(|s| s.params.len() == 1 && s.params[0].ty.as_deref() == Some("String"))
                .map(|s| (clone_params(&s.params), s.getter))
        });
        let Some((params, body)) = getter else {
            return Ok(None);
        };
        let args = vec![CallArg {
            label: None,
            value: SwiftValue::Str(member.to_string()),
            place: None,
        }];
        let saved_env = self.env.enter_isolated();
        self.env.declare("self", receiver.clone(), false);
        let bound = self.bind_params(&params, args);
        let result = match bound {
            Ok(_) => match body {
                Some(b) => self.eval(&b),
                None => Ok(SwiftValue::Void),
            },
            Err(e) => Err(e),
        };
        self.env.restore(saved_env);
        match result {
            Ok(v) | Err(Signal::Return(v)) => Ok(Some(v)),
            Err(e) => Err(e),
        }
    }

    /// The `@propertyWrapper` type of `field` on struct `type_name`, if any.
    pub(super) fn wrapped_field(&self, type_name: &str, field: &str) -> bool {
        self.structs
            .get(type_name)
            .is_some_and(|d| d.wrappers.contains_key(field))
    }

    /// Run `body` with `self` bound to `this` in a fresh scope, returning the
    /// body's value and the (possibly mutated) `self`.
    pub(super) fn run_with_self(
        &mut self,
        this: SwiftValue,
        body: impl FnOnce(&mut Self) -> Eval,
    ) -> Result<(SwiftValue, SwiftValue), Signal> {
        // Isolated from caller locals: a computed property/method body sees
        // globals, `self`, and its members — not enclosing variables.
        let saved_env = self.env.enter_isolated();
        self.env.declare("self", this, true);
        let result = body(self);
        let updated = self.env.get("self").unwrap_or(SwiftValue::Void);
        self.env.restore(saved_env);
        let value = match result {
            Ok(v) => v,
            Err(Signal::Return(v)) => v,
            Err(e) => return Err(e),
        };
        Ok((value, updated))
    }

    /// Set a property on a struct value, honoring computed setters and
    /// `willSet`/`didSet` observers. Returns the updated struct value.
    pub(super) fn set_struct_field(
        &mut self,
        value: SwiftValue,
        name: &str,
        new_value: SwiftValue,
    ) -> Result<SwiftValue, Signal> {
        let type_name = match &value {
            SwiftValue::Struct(o) => o.type_name.clone(),
            _ => return Err(EvalError::Type("cannot set a member on a non-struct".into()).into()),
        };

        // A wrapped property's set goes through its wrapper's `wrappedValue`.
        if self.wrapped_field(&type_name, name) {
            let current = match &value {
                SwiftValue::Struct(o) => o.get(name).cloned(),
                _ => None,
            };
            if let Some(wrapper) = current {
                let updated = self.set_struct_field(wrapper, "wrappedValue", new_value)?;
                let mut value = value;
                if let SwiftValue::Struct(obj) = &mut value {
                    Rc::make_mut(obj).set(name, updated);
                }
                return Ok(value);
            }
        }

        let setter = self
            .structs
            .get(&type_name)
            .and_then(|d| d.computed.get(name))
            .map(|c| (c.setter, c.setter_param.clone()));
        if let Some((Some(body), param)) = setter {
            let param = param.unwrap_or_else(|| "newValue".into());
            let nv = new_value.clone();
            let (_, updated) = self.run_with_self(value, |me| {
                me.env.declare(&param, nv, false);
                me.eval(&body)
            })?;
            return Ok(updated);
        }

        let observers = self.structs.get(&type_name).and_then(|d| {
            d.stored
                .iter()
                .find(|p| p.name == name)
                .map(|p| (p.will_set.clone(), p.did_set.clone()))
        });
        let (will_set, did_set) = observers.unwrap_or((None, None));
        let old_value = match &value {
            SwiftValue::Struct(o) => o.get(name).cloned(),
            _ => None,
        };

        let mut value = value;
        if let Some((param, body)) = will_set {
            let nv = new_value.clone();
            let (_, updated) = self.run_with_self(value, |me| {
                me.env.declare(&param, nv, false);
                me.eval(&body)
            })?;
            value = updated;
        }
        if let SwiftValue::Struct(obj) = &mut value {
            Rc::make_mut(obj).set(name, new_value);
        }
        if let Some((param, body)) = did_set {
            let old = old_value.unwrap_or(SwiftValue::Void);
            let (_, updated) = self.run_with_self(value, |me| {
                me.env.declare(&param, old, false);
                me.eval(&body)
            })?;
            value = updated;
        }
        Ok(value)
    }

    /// Assignment: plain `=` and compound `+=`, `-=`, … to a binding.
    pub(super) fn eval_assign(&mut self, node: &Node<'static>) -> Eval {
        let op = node
            .op_text()
            .ok_or_else(|| EvalError::Unsupported("assignment without operator".into()))?;
        let mut kids = node.children();
        let target = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("assignment without target".into()))?;
        let rhs = kids
            .next()
            .ok_or_else(|| EvalError::Unsupported("assignment without value".into()))?;

        // Tuple-destructuring assignment `(a, b) = (b, a + b)`: evaluate the
        // whole right side first (so swaps read the old values), then write each
        // element back through its own lvalue.
        if target.kind() == NodeKind::TupleExpr && op == "=" {
            let value = self.eval(&rhs)?;
            let targets: Vec<Node<'static>> = target.children().collect();
            self.assign_destructured(&targets, value)?;
            return Ok(SwiftValue::Void);
        }

        // Member assignment whose base is a class instance mutates in place
        // (reference semantics) rather than through a copy-on-write place.
        if target.kind() == NodeKind::MemberExpr {
            let field = target
                .text()
                .ok_or_else(|| EvalError::Unsupported("member assignment without a name".into()))?;
            let base = target
                .children()
                .next()
                .ok_or_else(|| EvalError::Unsupported("member assignment without a base".into()))?;
            // `Type.prop = value` — assign a type-level (static) stored property.
            if base.kind() == NodeKind::IdentExpr {
                if let Some(tn) = base.text() {
                    let key = format!("{tn}.{field}");
                    if self.env.get(&tn).is_none() && self.statics.contains_key(&key) {
                        let new_value = if op == "=" {
                            self.eval(&rhs)?
                        } else {
                            let current = self.statics[&key].clone();
                            let r = self.eval(&rhs)?;
                            ops::binary(op.trim_end_matches('='), &current, &r).map_err(trap)?
                        };
                        self.statics.insert(key, new_value);
                        return Ok(SwiftValue::Void);
                    }
                }
            }
            let base_value = self.eval(&base)?;
            if let SwiftValue::Object(obj) = &base_value {
                let new_value = if op == "=" {
                    self.eval(&rhs)?
                } else {
                    let current = self.read_object_member(&base_value, &field)?;
                    let r = self.eval(&rhs)?;
                    ops::binary(op.trim_end_matches('='), &current, &r).map_err(trap)?
                };
                self.set_object_field(obj, &field, new_value);
                return Ok(SwiftValue::Void);
            }
            // Subscript or struct member fall through to place-based handling.
        }

        // Subscript assignment `a[i] = v` over an array variable.
        if target.kind() == NodeKind::SubscriptExpr {
            return self.assign_subscript(&target, &rhs, &op);
        }

        // An unqualified static-property write inside a `static` method.
        if target.kind() == NodeKind::IdentExpr {
            if let Some(n) = target.text() {
                if self.env.get_local(&n).is_none() {
                    if let Some(key) = self.implicit_static_key(&n) {
                        let new_value = if op == "=" {
                            self.eval(&rhs)?
                        } else {
                            let current = self.statics[&key].clone();
                            let r = self.eval(&rhs)?;
                            ops::binary(op.trim_end_matches('='), &current, &r).map_err(trap)?
                        };
                        self.statics.insert(key, new_value);
                        return Ok(SwiftValue::Void);
                    }
                }
            }
        }

        // `self.<name>` where `self` is a class instance.
        if target.kind() == NodeKind::IdentExpr {
            if let Some(n) = target.text() {
                if self.env.get_local(&n).is_none() {
                    if let Some(SwiftValue::Object(obj)) = self.env.get("self") {
                        if self.class_has_member(&obj.borrow().class_name.clone(), &n) {
                            let new_value = if op == "=" {
                                self.eval(&rhs)?
                            } else {
                                let cur =
                                    self.read_object_member(&SwiftValue::Object(obj.clone()), &n)?;
                                let r = self.eval(&rhs)?;
                                ops::binary(op.trim_end_matches('='), &cur, &r).map_err(trap)?
                            };
                            self.set_object_field(&obj, &n, new_value);
                            return Ok(SwiftValue::Void);
                        }
                    }
                }
            }
        }

        // Resolve the target to an assignable place. A bare identifier that is
        // not a local binding but is a member of the current `self` becomes
        // `self.<name>`.
        let place = match self.resolve_place(&target) {
            Some(p) if p.path.is_empty() && self.env.get_local(&p.root).is_none() => {
                if self.self_has_member(&p.root) {
                    Place {
                        root: "self".into(),
                        path: vec![p.root],
                    }
                } else {
                    p
                }
            }
            Some(p) => p,
            None => {
                return Err(EvalError::Unsupported("unsupported assignment target".into()).into())
            }
        };

        let new_value = if op == "=" {
            self.eval(&rhs)?
        } else {
            let bin_op = op.trim_end_matches('=');
            let current = self.read_place(&place)?;
            let r = self.eval(&rhs)?;
            ops::binary(bin_op, &current, &r).map_err(trap)?
        };

        self.write_place(&place, new_value)?;
        Ok(SwiftValue::Void)
    }

    /// Read the current value stored at `place`.
    pub(super) fn read_place(&mut self, place: &Place) -> Eval {
        let mut value = self
            .env
            .get(&place.root)
            .or_else(|| self.statics.get(&place.root).cloned())
            .ok_or_else(|| EvalError::UnknownVariable(place.root.clone()))?;
        for field in &place.path {
            value = self.read_struct_member(&value, field)?;
        }
        Ok(value)
    }

    /// Whether the current `self` (if any) has a stored/computed member `name`.
    fn self_has_member(&self, name: &str) -> bool {
        match self.env.get("self") {
            Some(SwiftValue::Struct(obj)) => {
                obj.get(name).is_some() || self.struct_has_member(&obj.type_name, name)
            }
            _ => false,
        }
    }

    /// Member access: static integer members (`Int.max`/`Int.min`) and
    /// `Array.count`.
    /// Evaluate a `MemoryLayout<T>.size` / `.stride` / `.alignment` access.
    /// Layouts are modelled on a 64-bit platform. Primitive scalar types and
    /// user structs (laid out field-by-field with C-style alignment/padding)
    /// are supported; other types report an unsupported-feature error.
    fn memory_layout_member(&self, ty: &str, member: &str) -> Eval {
        let (size, stride, alignment) = self
            .type_layout(ty)
            .ok_or_else(|| EvalError::Unsupported(format!("MemoryLayout<{ty}>")))?;
        let pick = match member {
            "size" => size,
            "stride" => stride,
            "alignment" => alignment,
            other => {
                return Err(EvalError::Unsupported(format!("MemoryLayout<{ty}>.{other}")).into())
            }
        };
        Ok(SwiftValue::Int(IntValue::new(pick as i128, IntWidth::I64)))
    }

    /// The `(size, stride, alignment)` of `ty` on a 64-bit platform, or `None`
    /// if the type's layout is not modelled.
    fn type_layout(&self, ty: &str) -> Option<(u64, u64, u64)> {
        self.type_layout_inner(ty, &mut Vec::new())
    }

    /// `type_layout`, tracking the chain of structs currently being laid out so
    /// a recursive value type (`struct A { var a: A }`) fails safely instead of
    /// overflowing the stack.
    fn type_layout_inner(&self, ty: &str, stack: &mut Vec<String>) -> Option<(u64, u64, u64)> {
        // Scalar primitives: `(size, alignment)`; stride == size for these.
        let scalar = |n: u64| Some((n, n, n));
        match ty.trim() {
            "Int" | "UInt" | "Int64" | "UInt64" | "Double" | "Float64" => scalar(8),
            "Int32" | "UInt32" | "Float" | "Float32" => scalar(4),
            "Int16" | "UInt16" => scalar(2),
            "Int8" | "UInt8" | "Bool" => scalar(1),
            // An empty type still occupies a stride of 1.
            "Void" | "()" => Some((0, 1, 1)),
            other => self.struct_layout(other, stack),
        }
    }

    /// Compute a user struct's layout by laying out its stored properties in
    /// declaration order with C-style alignment and tail padding. A nested
    /// struct field advances the running offset by the field's *size* (its tail
    /// padding is reusable), matching Swift's value-type layout. Returns `None`
    /// for unmodelled field types or a self-referential (cyclic) layout.
    fn struct_layout(&self, type_name: &str, stack: &mut Vec<String>) -> Option<(u64, u64, u64)> {
        let def = self.structs.get(type_name)?;
        // A struct that (transitively) contains itself has no finite layout.
        if stack.iter().any(|t| t == type_name) {
            return None;
        }
        stack.push(type_name.to_string());
        let mut offset: u64 = 0;
        let mut max_align: u64 = 1;
        for prop in &def.stored {
            let Some(field_ty) = prop.ty.as_deref() else {
                stack.pop();
                return None;
            };
            let Some((fsize, _fstride, falign)) = self.type_layout_inner(field_ty, stack) else {
                stack.pop();
                return None;
            };
            max_align = max_align.max(falign);
            // Round the running offset up to the field's alignment.
            offset = offset.div_ceil(falign) * falign;
            offset += fsize;
        }
        stack.pop();
        let size = offset;
        // Stride rounds the size up to the struct's overall alignment.
        let stride = size.div_ceil(max_align) * max_align;
        Some((size, stride, max_align))
    }

    /// Resolve an implicit-member floating-point constant (`.infinity`, `.pi`,
    /// `.nan`, …) against the node's inferred or call-site contextual type when
    /// that type is a floating type (`Double`/`Float`/`CGFloat`). Returns `None`
    /// if the contextual type is not floating or the member is not a constant.
    fn resolve_implicit_float_constant(
        &self,
        node: &Node<'static>,
        member: &str,
    ) -> Option<SwiftValue> {
        let value = double_type_constant(member)?;
        let is_float_ty = |ty: &str| {
            ty.split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|t| matches!(t, "Double" | "Float" | "CGFloat"))
        };
        let contextual_is_float = node
            .type_name()
            .into_iter()
            .chain(self.contextual_type().map(String::from))
            .any(|ty| is_float_ty(&ty));
        contextual_is_float.then(|| SwiftValue::Double(value))
    }

    pub(super) fn eval_member(&mut self, node: &Node<'static>) -> Eval {
        let mut member = node
            .text()
            .ok_or_else(|| EvalError::Unsupported("member without a name".into()))?;

        // Shorthand `.case` (no base): construct the inferred enum case.
        let Some(base) = node.first_child() else {
            if member == "." {
                member = node.op_text().unwrap_or(member);
            }
            if let Some(tn) = self.resolve_member_enum(node, &member) {
                return Ok(self.make_enum_case(&tn, &member, Vec::new())?.unwrap());
            }
            // Implicit member of a static property: `.red` where the contextual
            // type declares `static let red`. Resolve via the node's inferred
            // type, else a unique static whose member name matches.
            // Implicit floating-point type constant: `.infinity`/`.pi`/`.nan`
            // where the contextual parameter type is `Double`/`Float`/`CGFloat`
            // (e.g. `.frame(maxWidth: .infinity)`). These constants are computed,
            // not registered statics, so `resolve_implicit_static` cannot see
            // them. Resolve them *before* the unique-static fallback inside
            // `resolve_implicit_static`, so an unrelated `Type.infinity` static
            // can never steal a contextually-floating `.infinity`.
            if let Some(v) = self.resolve_implicit_float_constant(node, &member) {
                return Ok(v);
            }
            if let Some(v) = self.resolve_implicit_static(node, &member) {
                return Ok(v);
            }
            return Err(EvalError::Unsupported(format!(".{member} (unresolved type)")).into());
        };

        if base.kind() == NodeKind::IdentExpr {
            if let Some(type_name) = base.text() {
                // `Self.member` resolves through the enclosing type. The keyword
                // is never a value binding, so it bypasses the env shadow check
                // below even if a local happens to share the resolved type name.
                let is_self_kw = type_name == "Self";
                let type_name = self.resolve_self_keyword(type_name);
                // A generic placeholder (`T.defaultValue`) resolves to its bound
                // concrete type for the current call.
                let type_name = self.resolve_type_alias(&type_name).unwrap_or(type_name);
                if is_self_kw || self.env.get(&type_name).is_none() {
                    // `MemoryLayout<T>.size` / `.stride` / `.alignment`. The
                    // written type `T` is recorded as a `TypeIdent` child of the
                    // `MemoryLayout` identifier by the parser.
                    if type_name == "MemoryLayout" {
                        if let Some(ty) = base
                            .children()
                            .find(|c| c.kind() == NodeKind::TypeRef)
                            .and_then(|c| c.text())
                        {
                            return self.memory_layout_member(&ty, &member);
                        }
                    }
                    // `Task.isCancelled` — the running task's cooperative
                    // cancellation flag (`false` outside any task).
                    if type_name == "Task" && member == "isCancelled" {
                        return Ok(SwiftValue::Bool(self.current_task_cancelled()));
                    }
                    // `Type.self` — a metatype value naming the type.
                    if member == "self" && self.is_type_name(&type_name) {
                        return Ok(SwiftValue::Metatype(type_name));
                    }
                    if let Some(w) = IntWidth::from_type_name(&type_name) {
                        return match member.as_str() {
                            "max" => Ok(SwiftValue::Int(IntValue::new(w.max(), w))),
                            "min" => Ok(SwiftValue::Int(IntValue::new(w.min(), w))),
                            _ => {
                                Err(EvalError::Unsupported(format!("{type_name}.{member}")).into())
                            }
                        };
                    }
                    // Floating-point type constants: `Double.pi`, `.infinity`,
                    // `.nan`, and the magnitude/ulp bounds. `Float` shares the
                    // f64 model here, so the same values answer for both.
                    if type_name == "Double" || type_name == "Float" {
                        if let Some(v) = double_type_constant(&member) {
                            return Ok(SwiftValue::Double(v));
                        }
                        // Integer-typed format statics. `Float` is modelled on
                        // f64 here but keeps its own IEEE single field widths.
                        let (exp_bits, sig_bits) = if type_name == "Float" {
                            (8, 23)
                        } else {
                            (11, 52)
                        };
                        match member.as_str() {
                            "exponentBitCount" => return Ok(SwiftValue::int(exp_bits)),
                            "significandBitCount" => return Ok(SwiftValue::int(sig_bits)),
                            _ => {}
                        }
                    }
                    // Static property of a struct or class type: `Type.prop`.
                    if self.structs.contains_key(&type_name)
                        || self.classes.contains_key(&type_name)
                    {
                        if let Some(v) = self.statics.get(&format!("{type_name}.{member}")) {
                            return Ok(v.clone());
                        }
                    }
                    // Static computed property: `static var prop { … }`.
                    if let Some(v) = self.read_static_computed(&type_name, &member)? {
                        return Ok(v);
                    }
                    // Enum case (no associated values) or `allCases`.
                    if self.enums.contains_key(&type_name) {
                        if member == "allCases" {
                            return self.enum_all_cases(&type_name);
                        }
                        if let Some(v) = self.make_enum_case(&type_name, &member, Vec::new())? {
                            return Ok(v);
                        }
                    }
                }
            }
        }

        let value = self.eval(&base)?;
        // Optional chaining: a nil base short-circuits the whole access to nil.
        if matches!(value, SwiftValue::Nil) {
            return Ok(SwiftValue::Nil);
        }
        // Task handle members: `.value`/`.result` keep the handle so the
        // enclosing `await` drives it; `.isCancelled` reads the flag (ADR-0005).
        if let SwiftValue::Task(tid) = &value {
            match member.as_str() {
                "value" | "result" => return Ok(value.clone()),
                "isCancelled" => return Ok(SwiftValue::Bool(self.task_cancelled(*tid))),
                _ => {}
            }
        }
        // Class instance members.
        if let SwiftValue::Object(_) = &value {
            return self.read_object_member(&value, &member);
        }
        // Enum members: rawValue and computed properties.
        if let SwiftValue::Enum(e) = &value {
            if member == "rawValue" {
                return self.enum_raw_value(&e.type_name, &e.case);
            }
            if let Some(v) = self.read_enum_computed(&value, &member)? {
                return Ok(v);
            }
        }
        if let SwiftValue::Struct(obj) = &value {
            // Lazy stored property: materialize on first access and cache it
            // back into the storage when the base is an lvalue.
            if obj.get(&member).is_none() {
                if let Some(def) = self.lazy_default(&obj.type_name, &member) {
                    let (computed, _) = self.run_with_self(value.clone(), |me| me.eval(&def))?;
                    if let Some(place) = self.resolve_place(&base) {
                        let cached =
                            self.set_struct_field(value.clone(), &member, computed.clone())?;
                        self.write_place(&place, cached)?;
                    }
                    return Ok(computed);
                }
            }
            if obj.get(&member).is_some() || self.struct_has_member(&obj.type_name, &member) {
                return self.read_struct_member(&value, &member);
            }
            if let Some(kind) = BuiltinReceiver::of(&value) {
                if let Some(func) = self.properties.get(&(kind, member.clone())).copied() {
                    return func(value).map_err(Self::std_error_to_signal);
                }
            }
            return self.read_struct_member(&value, &member);
        }
        // Standard-library computed-property intrinsics (`Double.isNaN`,
        // `Int.magnitude`, …) on builtin receivers.
        if let Some(kind) = BuiltinReceiver::of(&value) {
            if let Some(func) = self.properties.get(&(kind, member.clone())).copied() {
                return func(value).map_err(Self::std_error_to_signal);
            }
        }
        match (&value, member.as_str()) {
            // Array `count`/`isEmpty` are served by the property registry (S4).
            (SwiftValue::Str(s), "count") => Ok(SwiftValue::int(crate::graphemes(s).len() as i128)),
            (SwiftValue::Str(s), "isEmpty") => Ok(SwiftValue::Bool(s.is_empty())),
            (SwiftValue::Tuple(items, _), idx) if idx.parse::<usize>().is_ok() => {
                let i: usize = idx.parse().unwrap();
                items
                    .get(i)
                    .cloned()
                    .ok_or_else(|| EvalError::Type(format!("tuple index .{i} out of range")).into())
            }
            // Named tuple element access (`r.min` on `(min: 1, max: 9)`). This
            // also serves a dictionary element's `.key`/`.value`, since
            // `materialize_builtin_sequence` emits those tuple labels.
            (SwiftValue::Tuple(items, labels), name)
                if SwiftValue::tuple_label_index(labels, name).is_some() =>
            {
                let i = SwiftValue::tuple_label_index(labels, name).unwrap();
                Ok(items[i].clone())
            }
            _ => {
                // User extension computed property on a builtin type
                // (`extension Int { var isEven: Bool { … } }`).
                let tn = value.type_name();
                if let Some(body) = self
                    .builtin_ext_computed
                    .get(&tn)
                    .and_then(|m| m.get(member.as_str()))
                    .and_then(|c| c.getter)
                {
                    return self
                        .run_with_self(value.clone(), |me| me.eval(&body))
                        .map(|(v, _)| v);
                }
                Err(EvalError::Unsupported(format!("member .{member} on {tn}")).into())
            }
        }
    }

    /// Evaluate a key-path literal `\Root.a.b` into a `KeyPath` value. The root
    /// type (a leading `TypeRef` child) is only needed at type-check time; the
    /// runtime keeps the ordered list of component names. `\.self` (and an
    /// embedded `.self`) is the identity path and contributes no component.
    pub(super) fn eval_keypath(&mut self, node: &Node<'static>) -> Eval {
        let components: Vec<String> = node
            .children()
            .filter(|c| c.kind() == NodeKind::IdentExpr)
            .filter_map(|c| c.text())
            .filter(|n| n != "self")
            .collect();
        let id = self.closures.len();
        self.closures
            .push((ClosureDef::KeyPath(components), Vec::new()));
        Ok(SwiftValue::Closure(id))
    }

    /// The path components of a key-path closure value, if `value` is one.
    fn keypath_components(&self, value: &SwiftValue) -> Option<Vec<String>> {
        if let SwiftValue::Closure(id) = value {
            if let Some((ClosureDef::KeyPath(components), _)) = self.closures.get(*id) {
                return Some(components.clone());
            }
        }
        None
    }

    /// Read `root[keyPath: kp]` by walking each component in turn. A `nil`
    /// encountered mid-path short-circuits to `nil` (optional-chained access).
    pub(super) fn apply_keypath(&mut self, root: SwiftValue, components: &[String]) -> Eval {
        let mut value = root;
        for name in components {
            if matches!(value, SwiftValue::Nil) {
                return Ok(SwiftValue::Nil);
            }
            value = self.read_named_member(value, name)?;
        }
        Ok(value)
    }

    /// Read the member named `name` from an already-evaluated `value`. Shared by
    /// key-path traversal; mirrors the value-dispatch tail of `eval_member`
    /// (struct/class/enum members, plus builtin `count`/`isEmpty` and labelled
    /// tuple elements).
    fn read_named_member(&mut self, value: SwiftValue, name: &str) -> Eval {
        if matches!(value, SwiftValue::Nil) {
            return Ok(SwiftValue::Nil);
        }
        match &value {
            SwiftValue::Object(_) => self.read_object_member(&value, name),
            SwiftValue::Struct(_) => self.read_struct_member(&value, name),
            SwiftValue::Enum(e) => {
                if name == "rawValue" {
                    return self.enum_raw_value(&e.type_name, &e.case);
                }
                if let Some(v) = self.read_enum_computed(&value, name)? {
                    return Ok(v);
                }
                Err(
                    EvalError::Unsupported(format!("key-path member .{name} on {}", e.type_name))
                        .into(),
                )
            }
            _ => {
                if let Some(kind) = BuiltinReceiver::of(&value) {
                    if let Some(func) = self.properties.get(&(kind, name.to_string())).copied() {
                        return func(value).map_err(Self::std_error_to_signal);
                    }
                }
                match (&value, name) {
                    (SwiftValue::Str(s), "count") => {
                        Ok(SwiftValue::int(crate::graphemes(s).len() as i128))
                    }
                    (SwiftValue::Str(s), "isEmpty") => Ok(SwiftValue::Bool(s.is_empty())),
                    (SwiftValue::Tuple(items, labels), n)
                        if SwiftValue::tuple_label_index(labels, n).is_some() =>
                    {
                        let i = SwiftValue::tuple_label_index(labels, n).unwrap();
                        Ok(items[i].clone())
                    }
                    _ => Err(EvalError::Unsupported(format!(
                        "key-path member .{name} on {}",
                        value.type_name()
                    ))
                    .into()),
                }
            }
        }
    }

    /// Write `container[keyPath: kp] = value`, returning the updated container
    /// (value types are rebuilt copy-on-write; class instances mutate in place
    /// and are returned unchanged). An empty path is the identity path, so the
    /// whole value is replaced.
    fn set_keypath(
        &mut self,
        container: SwiftValue,
        components: &[String],
        value: SwiftValue,
    ) -> Eval {
        match components {
            [] => Ok(value),
            [name] => self.set_named_member(container, name, value),
            [name, rest @ ..] => {
                let child = self.read_named_member(container.clone(), name)?;
                let new_child = self.set_keypath(child, rest, value)?;
                self.set_named_member(container, name, new_child)
            }
        }
    }

    /// Set the member `name` on `container` to `value`. Structs are rebuilt via
    /// `set_struct_field` (copy-on-write); class instances are mutated through
    /// their shared storage.
    fn set_named_member(&mut self, container: SwiftValue, name: &str, value: SwiftValue) -> Eval {
        match &container {
            SwiftValue::Struct(_) => self.set_struct_field(container.clone(), name, value),
            SwiftValue::Object(obj) => {
                self.set_object_field(obj, name, value);
                Ok(container)
            }
            other => Err(EvalError::Type(format!(
                "cannot set key-path member .{name} on {}",
                other.type_name()
            ))
            .into()),
        }
    }

    /// Write the elements of `value` (a tuple) to a list of lvalue targets, as
    /// in tuple-destructuring assignment `(a, b) = (b, a + b)`.
    fn assign_destructured(
        &mut self,
        targets: &[Node<'static>],
        value: SwiftValue,
    ) -> Result<(), Signal> {
        let SwiftValue::Tuple(items, _) = value else {
            return Err(EvalError::Type(
                "tuple-destructuring assignment expects a tuple value".into(),
            )
            .into());
        };
        if items.len() != targets.len() {
            return Err(EvalError::Type(format!(
                "tuple pattern has {} elements but value has {}",
                targets.len(),
                items.len()
            ))
            .into());
        }
        for (t, v) in targets.iter().zip(items.iter().cloned()) {
            self.assign_destructured_one(t, v)?;
        }
        Ok(())
    }

    /// Assign one already-evaluated `value` to a single lvalue `target` (a
    /// destructuring-assignment element): a nested tuple, a wildcard discard, a
    /// class-instance member, or a place-based binding.
    fn assign_destructured_one(
        &mut self,
        target: &Node<'static>,
        value: SwiftValue,
    ) -> Result<(), Signal> {
        match target.kind() {
            NodeKind::TupleExpr => {
                let nested: Vec<Node<'static>> = target.children().collect();
                self.assign_destructured(&nested, value)
            }
            // `_` discards its element.
            NodeKind::WildcardPattern => Ok(()),
            NodeKind::IdentExpr if target.text().as_deref() == Some("_") => Ok(()),
            NodeKind::MemberExpr => {
                // A class-instance member mutates in place (reference semantics).
                if let Some(base) = target.first_child() {
                    let base_value = self.eval(&base)?;
                    if let SwiftValue::Object(obj) = &base_value {
                        let field = target.text().ok_or_else(|| {
                            EvalError::Unsupported("member assignment without a name".into())
                        })?;
                        self.set_object_field(obj, &field, value);
                        return Ok(());
                    }
                }
                let place = self.resolve_place(target).ok_or_else(|| {
                    EvalError::Unsupported("unsupported assignment target".into())
                })?;
                self.write_place(&place, value)
            }
            _ => {
                let place = self.resolve_place(target).ok_or_else(|| {
                    EvalError::Unsupported("unsupported assignment target".into())
                })?;
                self.write_place(&place, value)
            }
        }
    }

    pub(super) fn resolve_place(&self, node: &Node<'static>) -> Option<Place> {
        match node.kind() {
            NodeKind::IdentExpr => {
                let root = node.text()?;
                // A bare identifier that is not a local binding but names a
                // member of the enclosing `self` resolves as an implicit
                // `self.<name>` place (members shadow module globals), so
                // mutating-method writes flow back.
                if self.env.get_local(&root).is_none() {
                    if self.is_self_member(&root) {
                        return Some(Place {
                            root: "self".into(),
                            path: vec![root],
                        });
                    }
                    // An unqualified static property inside a `static` method
                    // becomes a place rooted at its `Type.name` static key, so
                    // mutating-method writes flow back to the static storage.
                    if let Some(key) = self.implicit_static_key(&root) {
                        return Some(Place {
                            root: key,
                            path: Vec::new(),
                        });
                    }
                }
                Some(Place {
                    root,
                    path: Vec::new(),
                })
            }
            NodeKind::MemberExpr => {
                let member = node.text()?;
                let base = node.first_child()?;
                let mut place = self.resolve_place(&base)?;
                place.path.push(member);
                Some(place)
            }
            _ => None,
        }
    }

    /// Whether the leaf member written by `path` resolves to a `nonmutating`
    /// computed setter on its containing struct. Used to decide that an
    /// immutable value-type root need not (and must not) be reassigned after the
    /// write, because the effect landed through a reference.
    fn leaf_setter_nonmutating(&mut self, root: &SwiftValue, path: &[String]) -> bool {
        let Some((leaf, parents)) = path.split_last() else {
            return false;
        };
        // Descend to the struct that directly holds the leaf member.
        let mut container = root.clone();
        for seg in parents {
            match self.read_struct_member(&container, seg) {
                Ok(v) => container = v,
                Err(_) => return false,
            }
        }
        let SwiftValue::Struct(obj) = &container else {
            return false;
        };
        self.structs
            .get(&obj.type_name)
            .and_then(|d| d.computed.get(leaf))
            .is_some_and(|c| c.setter_nonmutating)
    }

    /// Write `value` to the storage named by `place`, applying copy-on-write and
    /// any property observers at the leaf.
    pub(super) fn write_place(&mut self, place: &Place, value: SwiftValue) -> Result<(), Signal> {
        if place.path.is_empty() {
            // A static-property place is rooted at its `Type.name` key.
            if self.env.get(&place.root).is_none() && self.statics.contains_key(&place.root) {
                self.statics.insert(place.root.clone(), value);
                return Ok(());
            }
            return match self.env.assign(&place.root, value) {
                Ok(()) => Ok(()),
                Err(BindError::Immutable(n)) => Err(EvalError::Immutable(n).into()),
                Err(BindError::Unbound(n)) => Err(EvalError::UnknownVariable(n).into()),
            };
        }
        let root_val = self
            .env
            .get(&place.root)
            .or_else(|| self.statics.get(&place.root).cloned())
            .ok_or_else(|| EvalError::UnknownVariable(place.root.clone()))?;
        if self.env.get(&place.root).is_none() && self.statics.contains_key(&place.root) {
            let updated = self.set_in(root_val, &place.path, value)?;
            self.statics.insert(place.root.clone(), updated);
            return Ok(());
        }
        // A class instance is mutated in place through its shared storage, so
        // the root binding need not (and, for an immutable `self`, must not) be
        // reassigned — its identity is unchanged.
        let root_is_object = matches!(root_val, SwiftValue::Object(_));
        // A `nonmutating` computed setter at the leaf writes through a reference
        // (e.g. `Binding.wrappedValue` storing into a shared `_StateBox`),
        // leaving the value-type root unchanged — so there is nothing to write
        // back, and a `let` root must not be treated as an illegal mutation.
        let leaf_nonmutating = self.leaf_setter_nonmutating(&root_val, &place.path);
        let updated = self.set_in(root_val, &place.path, value)?;
        if root_is_object || leaf_nonmutating {
            return Ok(());
        }
        match self.env.assign(&place.root, updated) {
            Ok(()) => Ok(()),
            Err(BindError::Immutable(n)) => Err(EvalError::Immutable(n).into()),
            Err(BindError::Unbound(n)) => Err(EvalError::UnknownVariable(n).into()),
        }
    }

    /// Recursively set the value at `path` within `container`, honoring
    /// observers/computed setters at each struct level.
    fn set_in(&mut self, container: SwiftValue, path: &[String], value: SwiftValue) -> Eval {
        let (head, rest) = path.split_first().expect("non-empty path");
        // A class instance is mutated in place through its shared storage (its
        // identity is preserved), so writing a field — possibly nested through a
        // value member — does not rebuild the object.
        if let SwiftValue::Object(obj) = &container {
            let obj = obj.clone();
            if rest.is_empty() {
                self.set_object_field(&obj, head, value);
            } else {
                let sub = self.read_object_member(&container, head)?;
                let new_sub = self.set_in(sub, rest, value)?;
                self.set_object_field(&obj, head, new_sub);
            }
            return Ok(container);
        }
        if rest.is_empty() {
            return self.set_struct_field(container, head, value);
        }
        let sub = self.read_struct_member(&container, head)?;
        let new_sub = self.set_in(sub, rest, value)?;
        self.set_struct_field(container, head, new_sub)
    }
}

/// The value of a `Double`/`Float` type-level constant, if `member` names one.
/// Covers the `FloatingPoint` static properties the runtime models.
fn double_type_constant(member: &str) -> Option<f64> {
    Some(match member {
        "pi" => std::f64::consts::PI,
        "infinity" => f64::INFINITY,
        "nan" => f64::NAN,
        "greatestFiniteMagnitude" => f64::MAX,
        "leastNonzeroMagnitude" => f64::from_bits(1),
        "leastNormalMagnitude" => f64::MIN_POSITIVE,
        "ulpOfOne" => f64::EPSILON,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::double_type_constant;

    #[test]
    fn double_constants() {
        assert_eq!(double_type_constant("pi"), Some(std::f64::consts::PI));
        assert_eq!(double_type_constant("infinity"), Some(f64::INFINITY));
        assert!(double_type_constant("nan").unwrap().is_nan());
        assert!(double_type_constant("leastNonzeroMagnitude").unwrap() > 0.0);
        assert_eq!(double_type_constant("bogus"), None);
    }
}
