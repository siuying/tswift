use std::rc::Rc;

use tswift_frontend::{Node, NodeKind, TypeRepr};

use super::{
    autoclosure_flags, clone_params, literal_syntax_kind, materialize_sequence,
    max_shorthand_in_interpolations, param_type_hints, select_labeled_overload, trap, CallArg,
    ClosureDef, Eval, EvalError, Interpreter, Param, Place, Signal, MAX_CALL_DEPTH,
};
use crate::env::Env;
use crate::ops;
use crate::stdlib::{Arg, BuiltinReceiver, Outcome};
use crate::value::{EnumObj, SwiftValue};

impl<'w> Interpreter<'w> {
    /// Apply a stdlib method outcome, including mutating receiver write-back.
    fn apply_method_outcome(
        &mut self,
        outcome: Outcome,
        mutating: bool,
        base_place: Option<Place>,
    ) -> Eval {
        let Outcome { result, receiver } = outcome;
        if mutating {
            if let Some(place) = base_place {
                self.write_place(&place, receiver)?;
            }
        }
        Ok(result)
    }

    /// Dispatch a method call on a builtin receiver through the intrinsic
    /// registry, if one is registered. Returns `None` when no intrinsic matches
    /// so the caller can fall through to the existing ad-hoc paths.
    /// Dispatch a label-aware method call. `Ok(None)` from the stdlib handler
    /// means this label shape is not one of its overloads, so normal positional
    /// dispatch should continue.
    fn dispatch_labeled_intrinsic(
        &mut self,
        recv_value: SwiftValue,
        method: &str,
        args: Vec<Arg>,
        base_place: Option<Place>,
    ) -> Option<Eval> {
        let kind = BuiltinReceiver::of(&recv_value)?;
        let entry = self.builtins.labeled_intrinsic(kind, method)?;
        let outcome = (entry.func)(self, recv_value, args);
        match outcome {
            Ok(Some(outcome)) => {
                Some(self.apply_method_outcome(outcome, entry.mutating, base_place))
            }
            Ok(None) => None,
            Err(err) => Some(Err(Self::std_error_to_signal(err))),
        }
    }

    /// Find the most-derived method `name` for an object of `class_name`,
    /// returning the method and the class that declares it.
    pub(super) fn lookup_method(
        &self,
        class_name: &str,
        name: &str,
    ) -> Option<(Vec<Param>, Option<Node<'static>>, String, Vec<String>)> {
        let mut current = Some(class_name.to_string());
        while let Some(cls) = current {
            let def = self.types.class_def(&cls)?;
            if let Some(m) = def.methods.get(name) {
                return Some((
                    clone_params(&m.params),
                    m.body,
                    cls,
                    m.generic_params.clone(),
                ));
            }
            current = def.superclass.clone();
        }
        None
    }

    /// Invoke a closure value with call arguments, writing back any `inout`
    /// parameters to their caller locations (`f(&x)` over a closure whose
    /// parameter is `inout`). Falls back to the value-only path when the
    /// closure has no `inout` parameters.
    fn call_closure_with_args(&mut self, id: usize, args: Vec<CallArg>) -> Eval {
        // A closure participates in the `inout` write-back path when it either
        // declares an explicit `inout` parameter or is being called with an
        // `&`-prefixed argument. The latter covers shorthand closures
        // (`{ $0 += 1 }`), whose parameters carry no explicit `inout` marker —
        // the caller's `&x` is the contextual signal that position is `inout`.
        let has_inout = match self.closures.get(id) {
            Some((ClosureDef::User { params, .. }, _)) => {
                params.iter().any(|p| p.inout_) || args.iter().any(|a| a.place.is_some())
            }
            _ => false,
        };
        if !has_inout {
            let plain = args.into_iter().map(|a| a.value).collect();
            return self.call_closure(id, plain);
        }

        self.depth += 1;
        if self.depth > MAX_CALL_DEPTH {
            self.depth -= 1;
            return Err(trap("stack overflow: recursion too deep".into()));
        }
        let (params, body, captured) = match &self.closures[id] {
            (ClosureDef::User { params, body }, cap) => {
                (clone_params(params), body.clone(), cap.clone())
            }
            _ => unreachable!("operator/non-user closure has no inout params"),
        };
        let call_env = Env::with_captured(captured);
        let saved = std::mem::replace(&mut self.env, call_env);

        let mut writebacks: Vec<(String, Place)> = Vec::new();
        for (i, p) in params.iter().enumerate() {
            // Swift rejects passing a value to an `inout` parameter without an
            // explicit `&`; an `inout` parameter therefore requires its caller
            // argument to carry a write-back `Place`.
            if p.inout_ && args.get(i).and_then(|a| a.place.as_ref()).is_none() {
                self.env = saved;
                self.depth -= 1;
                return Err(trap(format!(
                    "passing value to 'inout' parameter '{}' requires '&'",
                    p.name
                )));
            }
            let v = args
                .get(i)
                .map(|a| a.value.clone())
                .unwrap_or(SwiftValue::Nil);
            let place = args.get(i).and_then(|a| a.place.clone());
            self.env.declare(&p.name, v, p.inout_ || place.is_some());
            if let Some(place) = place {
                writebacks.push((p.name.clone(), place));
            }
        }
        for (i, a) in args.iter().enumerate() {
            // Shorthand closures (`{ $0 += 1 }`) expose no named `Param`; an
            // `&`-passed argument makes the `$i` binding the mutable `inout`
            // target and schedules its final value for write-back.
            let is_shorthand_inout = i >= params.len() && a.place.is_some();
            self.env
                .declare(&format!("${i}"), a.value.clone(), is_shorthand_inout);
            if is_shorthand_inout {
                if let Some(place) = a.place.clone() {
                    writebacks.push((format!("${i}"), place));
                }
            }
        }

        let mut result = Ok(SwiftValue::Void);
        for stmt in &body {
            match self.eval(stmt) {
                Ok(v) => result = Ok(v),
                Err(Signal::Return(v)) => {
                    result = Ok(v);
                    break;
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        // Capture the final value of each `inout` parameter before unwinding.
        let finals: Vec<(Place, SwiftValue)> = writebacks
            .iter()
            .filter_map(|(name, place)| self.env.get(name).map(|v| (place.clone(), v)))
            .collect();
        self.env = saved;
        self.depth -= 1;
        // Swift copies `inout` arguments back even when the callee throws, so
        // mutations are visible after a caught/`try?`-converted error. Only a
        // fatal interpreter trap (`Signal::Error`) skips the copy-out.
        if !matches!(result, Err(Signal::Error(_))) {
            for (place, v) in finals {
                self.write_place(&place, v)?;
            }
        }
        result
    }

    /// The greatest shorthand-argument index (`$0`, `$1`, …) referenced anywhere
    /// in `node`'s subtree, used to decide a closure's implicit arity for
    /// tuple-splat shorthand binding. `None` when no `$N` shorthand appears.
    fn max_shorthand(node: &Node<'static>) -> Option<usize> {
        let here = match node.kind() {
            NodeKind::IdentExpr => node
                .text()
                .as_deref()
                .and_then(|t| t.strip_prefix('$'))
                .and_then(|d| d.parse::<usize>().ok()),
            // `$0`/`$1` inside a string-interpolation segment (`"\($1)"`) are not
            // separate AST nodes — the interpolation is re-parsed lazily — so
            // scan the literal's interpolation text for shorthand references.
            NodeKind::StringLiteral => node
                .text()
                .as_deref()
                .and_then(max_shorthand_in_interpolations),
            _ => None,
        };
        node.children()
            .filter_map(|c| Self::max_shorthand(&c))
            .chain(here)
            .max()
    }

    /// Bind a user closure's named parameters and `$N` shorthands consistently
    /// for both normal and result-builder closure evaluation.
    pub(super) fn bind_closure_args(
        &mut self,
        params: &[Param],
        body: &[Node<'static>],
        args: &[SwiftValue],
    ) {
        let shorthand_args = Self::shorthand_args(params, body, args);
        for (i, p) in params.iter().enumerate() {
            let v = args.get(i).cloned().unwrap_or(SwiftValue::Nil);
            self.env.declare(&p.name, v, false);
        }
        for (i, v) in shorthand_args.iter().enumerate() {
            self.env.declare(&format!("${i}"), v.clone(), false);
        }
    }

    /// Tuple-splat shorthand: a closure that references `$1`, `$2`, … but is
    /// called with a single tuple argument destructures the tuple across the
    /// shorthands. A closure using only `$0` keeps the whole tuple as `$0`.
    fn shorthand_args(
        params: &[Param],
        body: &[Node<'static>],
        args: &[SwiftValue],
    ) -> Vec<SwiftValue> {
        match args {
            [SwiftValue::Tuple(elems, _)] if params.is_empty() => {
                let arity = body.iter().map(Self::max_shorthand).max().flatten();
                match arity {
                    Some(n) if n >= 1 && n + 1 == elems.len() => elems.clone(),
                    _ => args.to_vec(),
                }
            }
            _ => args.to_vec(),
        }
    }

    /// Invoke a closure value with already-evaluated arguments.
    pub(super) fn call_closure(&mut self, id: usize, args: Vec<SwiftValue>) -> Eval {
        if id >= self.closures.len() {
            return Err(EvalError::UnknownFunction("<closure>".into()).into());
        }
        self.depth += 1;
        if self.depth > MAX_CALL_DEPTH {
            self.depth -= 1;
            return Err(trap("stack overflow: recursion too deep".into()));
        }
        // An operator-function reference applies its operator directly to the
        // arguments (binary for two, unary for one) without a call frame.
        if let (ClosureDef::Operator(op), _) = &self.closures[id] {
            let op = op.clone();
            self.depth -= 1;
            return match args.as_slice() {
                [a, b] => ops::binary(&op, a, b).map_err(trap),
                [a] => ops::unary(&op, a).map_err(trap),
                _ => Err(EvalError::Unsupported(format!(
                    "operator `{op}` reference expects 1 or 2 arguments"
                ))
                .into()),
            };
        }
        // A key-path value used as a function: walk the path from its single
        // argument (`names.map(\.count)`).
        if let (ClosureDef::KeyPath(components), _) = &self.closures[id] {
            let components = components.clone();
            self.depth -= 1;
            let [root] = <[SwiftValue; 1]>::try_from(args).map_err(|args| {
                EvalError::Unsupported(format!(
                    "key-path function expects exactly one argument, got {}",
                    args.len()
                ))
            })?;
            return self.apply_keypath(root, &components);
        }
        let (params, body, captured) = {
            let (def, cap) = &self.closures[id];
            match def {
                ClosureDef::User { params, body } => {
                    (clone_params(params), body.clone(), cap.clone())
                }
                ClosureDef::Operator(_) | ClosureDef::KeyPath(_) => {
                    unreachable!("operator/key-path handled above")
                }
            }
        };
        let call_env = Env::with_captured(captured);
        let saved = std::mem::replace(&mut self.env, call_env);

        self.bind_closure_args(&params, &body, &args);

        // Evaluate the closure body statements, yielding the last value.
        let mut result = Ok(SwiftValue::Void);
        for stmt in &body {
            match self.eval(stmt) {
                Ok(v) => result = Ok(v),
                Err(Signal::Return(v)) => {
                    result = Ok(v);
                    break;
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.env = saved;
        self.depth -= 1;
        result
    }

    /// Invoke a user function by its table id with (possibly labeled) arguments.
    pub(super) fn call_function(&mut self, id: usize, args: Vec<CallArg>) -> Eval {
        if id >= self.funcs.len() {
            return Err(EvalError::UnknownFunction("<function value>".into()).into());
        }
        self.depth += 1;
        if self.depth > MAX_CALL_DEPTH {
            self.depth -= 1;
            return Err(trap(
                "stack overflow: recursion exceeded the maximum call depth".into(),
            ));
        }

        // Bind parameters in a fresh scope over the function's captured chain.
        let params = clone_params(&self.funcs[id].params);
        let body = self.funcs[id].body;
        let captured = self.funcs[id].captured.clone();
        let generics = self.funcs[id].generic_params.clone();
        let call_env = Env::with_captured(captured);
        let saved = std::mem::replace(&mut self.env, call_env);

        // Bind generic type parameters to the concrete types of the arguments
        // so a static reference through a placeholder (`T.zero()`) resolves.
        let type_binding = self.infer_type_bindings(&generics, &params, &args);
        self.type_bindings.push(type_binding);

        let bound = self.bind_params(&params, args);
        let outcome = match bound {
            Ok(inout_binds) => {
                let result = match body {
                    Some(b) => match self.eval(&b) {
                        Ok(v) => Ok(v),
                        Err(Signal::Return(v)) => Ok(v),
                        Err(other) => Err(other),
                    },
                    None => Ok(SwiftValue::Void),
                };
                // Capture inout finals before tearing down the call scope.
                let writes: Vec<(Place, SwiftValue)> = inout_binds
                    .iter()
                    .filter_map(|(name, place)| self.env.get(name).map(|v| (place.clone(), v)))
                    .collect();
                result.map(|v| (v, writes))
            }
            Err(e) => Err(e),
        };

        self.type_bindings.pop();
        self.env = saved;
        self.depth -= 1;

        let (mut value, writes) = outcome?;
        for (place, val) in writes {
            self.write_place(&place, val)?;
        }
        // Apply the declared tuple return labels so `f().lo` resolves even when
        // the returned tuple literal carried no labels of its own.
        if let (SwiftValue::Tuple(items, labels), Some(decl)) =
            (&mut value, &self.funcs[id].ret_tuple_labels)
        {
            if items.len() == decl.len() && labels.iter().all(Option::is_none) {
                *labels = decl.clone();
            }
        }
        Ok(value)
    }

    /// Evaluate a call: a method, a struct initializer, a user function, a
    /// native, or a conversion initializer.
    pub(super) fn eval_call(&mut self, node: &Node<'static>) -> Eval {
        let children: Vec<Node<'static>> = node.children().collect();
        let callee = children
            .first()
            .ok_or_else(|| EvalError::Unsupported("call with no callee".into()))?;
        let arg_nodes = &children[1..];

        // Method call: `base.method(args)`.
        if callee.kind() == NodeKind::MemberExpr {
            return self.eval_method_call(callee, arg_nodes);
        }

        // A *type* literal applied to arguments is a collection constructor:
        // `[Int]()`, `[String: Int]()`, `[Int](repeating: 0, count: 3)`. The
        // callee literal is not evaluated (its elements are type names, not
        // values). A value literal such as `[1, 2]()` is not a type literal and
        // falls through to normal (erroring) evaluation.
        if matches!(
            callee.kind(),
            NodeKind::ArrayLiteral | NodeKind::DictLiteral
        ) && self.is_type_literal_callee(callee)
        {
            let args = self.eval_args_with(arg_nodes, None)?;
            if callee.kind() == NodeKind::DictLiteral {
                if !args.is_empty() {
                    return Err(EvalError::Type("[K: V](...) takes no arguments".into()).into());
                }
                return Ok(SwiftValue::Dict(Rc::new(Vec::new())));
            }
            return self.construct_array_literal_ctor(&args);
        }

        // Structured-concurrency entry points (ADR-0005). Handled before
        // argument evaluation so a metatype label like `of: Int.self` is never
        // eagerly evaluated; only the trailing body closure matters.
        if callee.kind() == NodeKind::IdentExpr {
            if let Some(name) = callee.text() {
                if let Some(v) = self.try_concurrency_builtin(&name, arg_nodes)? {
                    return Ok(v);
                }
            }
        }

        // If the callee is a known user function with `@autoclosure` params,
        // defer those argument expressions into thunks (capturing this scope).
        // A builtin free function (e.g. a SwiftUI view constructor) may instead
        // carry a declared parameter signature, used to push a contextual type
        // so a leading-dot member argument resolves against the parameter type.
        let call_params = if callee.kind() == NodeKind::IdentExpr {
            callee.text().and_then(|name| match self.env.get(&name) {
                Some(SwiftValue::Function(id)) => Some(clone_params(&self.funcs[id].params)),
                // Any other binding shadows the builtin — do not push its hints.
                Some(_) => None,
                // An unbound name resolves to a builtin free fn *only* when no
                // user type of the same name shadows it (a user `struct VStack`
                // dispatches to its own initializer, so it must not inherit the
                // builtin `VStack`'s parameter hints).
                None => {
                    if self.is_user_nominal_type(&name) {
                        None
                    } else {
                        self.globals
                            .free_fn(&name)
                            .and_then(|e| e.params.as_ref())
                            .map(|p| clone_params(p))
                    }
                }
            })
        } else {
            None
        };
        // An optional-chained call on an absent callee (`completion?()` with
        // `completion == nil`) short-circuits *before* argument evaluation —
        // Swift skips the arguments' side effects when the chain is nil.
        if callee.kind() == NodeKind::IdentExpr {
            if let Some(name) = callee.text() {
                if matches!(self.env.get(&name), Some(SwiftValue::Nil)) {
                    return Ok(SwiftValue::Nil);
                }
            }
        }
        let args = self.eval_args_with(arg_nodes, call_params.as_deref())?;

        if callee.kind() == NodeKind::IdentExpr {
            let name = callee
                .text()
                .ok_or_else(|| EvalError::Unsupported("unnamed callee".into()))?;
            // `Self(...)` constructs an instance of the enclosing type.
            let name = self.resolve_self_keyword(name);

            // `type(of: x)` — the dynamic type of `x` as a metatype value.
            if name == "type" && self.is_unshadowed("type") {
                if let Some(arg) = args
                    .iter()
                    .find(|a| a.label.as_deref() == Some("of"))
                    .or_else(|| args.first())
                {
                    return Ok(SwiftValue::Metatype(arg.value.type_name()));
                }
            }
            // `EnumType(rawValue:)` — failable lookup of the case with that raw
            // value, returning the case or `nil` (RawRepresentable synthesis).
            if self.types.is_enum(&name) {
                if let Some(raw) = args
                    .iter()
                    .find(|a| a.label.as_deref() == Some("rawValue"))
                    .map(|a| a.value.clone())
                {
                    let case = self
                        .types
                        .enum_def(&name)
                        .unwrap()
                        .cases
                        .iter()
                        .find(|c| c.raw.as_ref() == Some(&raw))
                        .map(|c| c.name.clone());
                    return Ok(match case {
                        Some(name_) => SwiftValue::Enum(Rc::new(EnumObj {
                            type_name: name.clone(),
                            case: name_,
                            payload: Vec::new(),
                        })),
                        None => SwiftValue::Nil,
                    });
                }
            }
            // Class initializer.
            if self.types.is_class(&name) {
                return self.instantiate_class(&name, args);
            }
            // Struct memberwise initializer. A specialization with integer
            // generic arguments (`Buf<4>()`) binds them, in order, to the
            // struct's `let` generic parameters.
            if self.types.is_struct(&name) {
                let simple: Vec<(Option<String>, SwiftValue)> = args
                    .iter()
                    .map(|a| (a.label.clone(), a.value.clone()))
                    .collect();
                let value_params = self
                    .types
                    .struct_def(&name)
                    .map(|d| d.value_generic_params.clone())
                    .unwrap_or_default();
                if !value_params.is_empty() {
                    let ints: Vec<SwiftValue> = callee
                        .children()
                        .filter(|c| c.kind() == NodeKind::TypeRef)
                        .filter_map(|c| c.text())
                        .filter_map(|t| parse_int_generic_arg(&t))
                        .map(SwiftValue::int)
                        .collect();
                    if ints.len() != value_params.len() {
                        return Err(EvalError::Type(format!(
                            "{name} expects {} integer generic argument(s), got {}",
                            value_params.len(),
                            ints.len()
                        ))
                        .into());
                    }
                    let type_values: Vec<(String, SwiftValue)> =
                        value_params.into_iter().zip(ints).collect();
                    return self.instantiate_struct_specialized(&name, &simple, &type_values);
                }
                return self.instantiate_struct(&name, &simple);
            }
            // `@dynamicCallable`: calling a struct instance routes through its
            // `dynamicallyCall(...)` method.
            if let Some(value @ SwiftValue::Struct(_)) = self.env.get(&name) {
                if self.is_dynamic_callable(&value) {
                    return self.dynamic_call(value, args);
                }
            }
            // A bound function or closure value (incl. recursion). A computed
            // variable holding a function value runs its getter first.
            let callee_binding = match self.env.get(&name) {
                Some(SwiftValue::AccessorVar(idx)) => Some(self.read_accessor_var(idx)?),
                other => other,
            };
            match callee_binding {
                Some(SwiftValue::Function(id)) => return self.call_function(id, args),
                Some(SwiftValue::Closure(id)) => {
                    return self.call_closure_with_args(id, args);
                }
                // An optional-chained call on an absent callee (`completion?()`
                // with `completion == nil`) evaluates to nil. The parser drops
                // the `?` like it does for `?.`, so an absent binding in call
                // position nil-propagates here.
                Some(SwiftValue::Nil) => return Ok(SwiftValue::Nil),
                _ => {}
            }
            // `swap(&a, &b)` — exchange two inout locations. Needs the caller
            // write-back `Place`s, so it cannot ride the value-only free-fn seam.
            if name == "swap" && self.is_unshadowed("swap") && args.len() == 2 {
                if let (Some(pa), Some(pb)) = (args[0].place.clone(), args[1].place.clone()) {
                    let va = args[0].value.clone();
                    let vb = args[1].value.clone();
                    self.write_place(&pa, vb)?;
                    self.write_place(&pb, va)?;
                    return Ok(SwiftValue::Void);
                }
            }
            // `isKnownUniquelyReferenced(&obj)` — true when the class instance is
            // not shared. The env binding plus this evaluated clone account for
            // two strong references, so a unique object reads as exactly two.
            if name == "isKnownUniquelyReferenced"
                && self.is_unshadowed("isKnownUniquelyReferenced")
                && args.len() == 1
            {
                return Ok(match &args[0].value {
                    SwiftValue::Object(rc) => SwiftValue::Bool(Rc::strong_count(rc) == 2),
                    _ => SwiftValue::Bool(false),
                });
            }

            // Core-internal value-only builtin constructors: generic collection
            // ctors (`Array`/`Set`/`Dictionary`/…), scalar conversion
            // initializers, and the `JSONEncoder`/`JSONDecoder` markers. The
            // table is consulted once, *after* user-type and binding dispatch
            // and gated by the shadow check, so a same-named user `struct`,
            // `enum`, `class`, or binding wins (correct Swift shadowing — and a
            // fix for the JSON markers, which were formerly matched before user
            // types with no shadow guard). Each entry returns `None` to fall
            // through to the rest of the ladder.
            if self.is_unshadowed(&name) {
                if let Some(ctor) = self.builtin_ctors.ctor(&name) {
                    if let Some(v) = ctor(self, &name, &args)? {
                        return Ok(v);
                    }
                }
            }

            // Free-function intrinsic served through the StdContext seam.
            if let Some(free) = self.globals.free_fn(&name).map(|e| e.f) {
                let labeled: Vec<Arg> = args.into_iter().map(Arg::from).collect();
                return free(self, labeled).map_err(Self::std_error_to_signal);
            }
            if let Some(native) = self.globals.native(&name) {
                let plain: Vec<SwiftValue> = args.into_iter().map(|a| a.value).collect();
                return Ok(native(self.out, &plain));
            }
            // An unqualified call inside a method resolves to `self.<name>()`.
            if let Some(this) = self.env.get("self") {
                match &this {
                    SwiftValue::Object(obj) => {
                        let cls = obj.borrow().class_name.clone();
                        if self.lookup_method(&cls, &name).is_some()
                            || self.protocol_default_method(&cls, &name).is_some()
                        {
                            return self.dispatch_class_method(this, &cls, &name, args);
                        }
                    }
                    SwiftValue::Struct(o) => {
                        let tn = o.type_name.clone();
                        if self.type_has_method(&tn, &name) {
                            let place = Place {
                                root: "self".into(),
                                path: vec![],
                            };
                            return self.call_struct_method(this, &tn, &name, args, Some(place));
                        }
                    }
                    SwiftValue::Enum(e) => {
                        let tn = e.type_name.clone();
                        if self.type_has_method(&tn, &name) {
                            return self.call_struct_method(this, &tn, &name, args, None);
                        }
                    }
                    _ => {}
                }
            }
            return Err(EvalError::UnknownFunction(name).into());
        }

        // Callee is an arbitrary expression — must evaluate to a callable value.
        let value = self.eval(callee)?;
        match value {
            SwiftValue::Function(id) => self.call_function(id, args),
            SwiftValue::Closure(id) => self.call_closure_with_args(id, args),
            other => {
                Err(EvalError::Type(format!("`{}` is not callable", other.type_name())).into())
            }
        }
    }

    /// Evaluate call arguments, resolving `inout` (`&place`) into a write-back
    /// location.
    pub(super) fn eval_args(
        &mut self,
        arg_nodes: &[Node<'static>],
    ) -> Result<Vec<CallArg>, Signal> {
        self.eval_args_with(arg_nodes, None)
    }

    /// Evaluate call arguments, deferring any that map to an `@autoclosure`
    /// parameter into a zero-argument thunk closure (capturing the caller's
    /// scope) instead of evaluating them eagerly.
    fn eval_args_with(
        &mut self,
        arg_nodes: &[Node<'static>],
        params: Option<&[Param]>,
    ) -> Result<Vec<CallArg>, Signal> {
        let autoclosure = params.map(|p| autoclosure_flags(p, arg_nodes));
        // Expected parameter type per argument, so an implicit-member argument
        // can resolve against the call-site contextual type.
        let hints = params.map(|p| param_type_hints(p, arg_nodes));
        let mut args = Vec::new();
        for (i, arg) in arg_nodes.iter().enumerate() {
            let label = arg.arg_label();
            if autoclosure.as_ref().is_some_and(|f| f[i]) {
                // `@autoclosure`: wrap the unevaluated expression in a thunk.
                let captured = self.env.capture();
                let id = self.closures.len();
                self.closures.push((
                    ClosureDef::User {
                        params: Vec::new(),
                        body: vec![*arg],
                    },
                    captured,
                ));
                args.push(CallArg {
                    label,
                    value: SwiftValue::Closure(id),
                    place: None,
                });
                continue;
            }
            let hint = hints.as_ref().and_then(|h| h[i].clone());
            // Pack expansion at a call site (`f(repeat each pack)`): the pack
            // array splats into individual positional arguments, so a pack
            // forwards through another variadic/pack parameter faithfully.
            if arg.kind() == NodeKind::PrefixExpr && arg.text().as_deref() == Some("repeat each") {
                if let SwiftValue::Array(items) = self.eval(arg)? {
                    for item in items.iter() {
                        args.push(CallArg {
                            label: None,
                            value: item.clone(),
                            place: None,
                        });
                    }
                    continue;
                }
                return Err(
                    EvalError::Type("`repeat each` expects a parameter pack".into()).into(),
                );
            }
            if arg.kind() == NodeKind::InoutExpr {
                let inner = arg
                    .children()
                    .next()
                    .ok_or_else(|| EvalError::Unsupported("inout without an lvalue".into()))?;
                let place = self.resolve_place(&inner);
                self.type_hint.push(hint.clone());
                let value = self.eval(&inner);
                self.type_hint.pop();
                let value = value?;
                args.push(CallArg {
                    label,
                    value,
                    place,
                });
            } else {
                self.type_hint.push(hint.clone());
                let value = self.eval(arg);
                self.type_hint.pop();
                let mut value = value?;
                if let (Some(ty), Some(kind)) = (hint.as_deref(), literal_syntax_kind(arg)) {
                    let repr = TypeRepr::parse(ty);
                    let optional = repr.is_optional();
                    if !(optional && kind == NodeKind::NilLiteral) {
                        value =
                            self.coerce_literal_value(repr.strip_optionals().text(), kind, value)?;
                    }
                }
                args.push(CallArg {
                    label,
                    value,
                    place: None,
                });
            }
        }
        Ok(args)
    }

    /// `base.method(args)`. Binds `self`; for `mutating` methods, writes the
    /// updated `self` back to `base`'s storage.
    /// Resolve and invoke a method whose selection is driven by the receiver's
    /// [`BuiltinReceiver`] kind: a label-aware overload (`layer 1a`), a
    /// type-specific intrinsic (`layer 1`, e.g. `Array.append`), or a
    /// `Sequence`/`Collection` algorithm (`layer 2`, e.g. `map`/`sorted`).
    /// Returns `Ok(None)` when none matches so the caller falls through the rest
    /// of the dispatch ladder. Concentrates the three ordered checks (and their
    /// argument-evaluation policies) behind one seam.
    /// Dispatch an `async` `AsyncSequence` algorithm (`reduce`, `map`, `filter`,
    /// `compactMap`, `flatMap`, `contains`, `allSatisfy`, `first`, `prefix`,
    /// `dropFirst`) on a custom sequence by collecting its elements
    /// eagerly and re-dispatching the call on the resulting array. Returns `None`
    /// when `base_value` is not a custom `AsyncSequence` or `method` is not one
    /// of these algorithms, so normal resolution continues.
    fn try_async_sequence_method(
        &mut self,
        base_value: &SwiftValue,
        base: &Node<'static>,
        method: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        const ASYNC_ALGOS: &[&str] = &[
            "reduce",
            "map",
            "filter",
            "compactMap",
            "flatMap",
            "contains",
            "allSatisfy",
            "first",
            "prefix",
            "dropFirst",
        ];
        if !ASYNC_ALGOS.contains(&method) {
            return Ok(None);
        }
        // The `makeStream(of:)` reader is an `AsyncStream`; a user nominal type
        // must declare `AsyncSequence` conformance.
        let is_async_seq = matches!(base_value, SwiftValue::AsyncStreamHandle(_))
            || self
                .value_type_name(base_value)
                .is_some_and(|ty| self.all_protocols(&ty).iter().any(|p| p == "AsyncSequence"));
        if !is_async_seq {
            return Ok(None);
        }
        let items = self.collect_async_sequence(base_value)?;
        let array = SwiftValue::Array(Rc::new(items));
        self.try_builtin_receiver_method(&array, base, method, arg_nodes)
    }

    fn try_builtin_receiver_method(
        &mut self,
        base_value: &SwiftValue,
        base: &Node<'static>,
        method: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        let mut evaluated_args = None;

        // Label-aware stdlib overloads (layer 1a): selected APIs need argument
        // labels to choose between overloads without leaking that policy into the
        // interpreter dispatcher.
        if let Some(kind) = BuiltinReceiver::of(base_value) {
            if self.builtins.has_labeled_intrinsic(kind, method) {
                let args = self.eval_args(arg_nodes)?;
                let labeled: Vec<Arg> = args.iter().map(Arg::from).collect();
                let place = self.resolve_place(base);
                if let Some(result) =
                    self.dispatch_labeled_intrinsic(base_value.clone(), method, labeled, place)
                {
                    return result.map(Some);
                }
                evaluated_args = Some(args);
            }
        }

        // Standard-library intrinsic registry (layer 1): type-specific members
        // such as `Array.append`. Consulted before the ad-hoc algorithm paths.
        if let Some(kind) = BuiltinReceiver::of(base_value) {
            if let Some(entry) = self.builtins.intrinsic(kind, method) {
                let args = match evaluated_args.take() {
                    Some(args) => args,
                    None => self.eval_args(arg_nodes)?,
                };
                // `IndexPath`/`IndexSet` intrinsics take positional arguments.
                // Exceptions: a handful of IndexSet methods are label-sensitive.
                if matches!(kind, BuiltinReceiver::IndexPath | BuiltinReceiver::IndexSet) {
                    let is_update = kind == BuiltinReceiver::IndexSet && method == "update";
                    // Methods that carry specific argument labels:
                    let is_intersects = kind == BuiltinReceiver::IndexSet && method == "intersects";
                    let is_shift = kind == BuiltinReceiver::IndexSet && method == "shift";
                    let is_filtered =
                        kind == BuiltinReceiver::IndexSet && method == "filteredIndexSet";
                    let labels_valid = args.iter().enumerate().all(|(i, arg)| {
                        match arg.label.as_deref() {
                            Some("with") => is_update,
                            Some("integersIn") => is_intersects,
                            Some("startingAt") => is_shift && i == 0,
                            Some("by") => is_shift && i == 1,
                            Some("includeInteger") => is_filtered,
                            Some(_) => false,
                            // Trailing-closure syntax passes filteredIndexSet's
                            // closure without a label; allow unlabeled for it.
                            None => !is_update && !is_intersects && !is_shift,
                        }
                    });
                    if !labels_valid {
                        return Err(EvalError::Type(format!(
                            "{}.{} called with unexpected argument label(s)",
                            kind.type_name(),
                            method
                        ))
                        .into());
                    }
                }
                let plain: Vec<SwiftValue> = args.into_iter().map(|a| a.value).collect();
                let place = self.resolve_place(base);
                return match (entry.func)(self, base_value.clone(), plain) {
                    Ok(outcome) => self.apply_method_outcome(outcome, entry.mutating, place),
                    Err(err) => Err(Self::std_error_to_signal(err)),
                }
                .map(Some);
            }
        }

        // Standard-library algorithm layer (layer 2): `Sequence`/`Collection`
        // methods (`map`/`filter`/`sorted`/…) over any builtin sequence.
        if self.globals.has_algorithm(method) {
            let items = if let Some(items) = materialize_sequence(base_value) {
                Some(items)
            } else if self.is_custom_sequence(base_value) {
                Some(self.materialize_custom_sequence(base_value.clone())?)
            } else {
                None
            };
            if let Some(items) = items {
                let func = self.globals.algorithm(method).unwrap();
                let labeled: Vec<Arg> = self
                    .eval_args(arg_nodes)?
                    .into_iter()
                    .map(Arg::from)
                    .collect();
                return func(self, items, labeled)
                    .map(Some)
                    .map_err(Self::std_error_to_signal);
            }
        }

        Ok(None)
    }

    /// Resolve and invoke a `Type.member(...)` call where `base` names a type
    /// (after `Self`/generic-alias substitution): a builtin static method
    /// (`Bool.random()`), a nested-type constructor (`Outer.Nested(...)`), an
    /// enum case, or a static struct/class method. Returns `Ok(None)` when
    /// `base` is not a type reference or names no such member, so the caller
    /// falls through to evaluating `base` as a value.
    fn try_type_qualified_method(
        &mut self,
        base: &Node<'static>,
        method: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        if base.kind() != NodeKind::IdentExpr {
            return Ok(None);
        }
        let Some(tn) = base.text() else {
            return Ok(None);
        };
        // `Self.method(...)` calls a static method of the enclosing type (the
        // keyword is never a value binding); any other name is a type reference
        // only when no local value shadows it.
        let Some(reference) = self.resolve_type_reference(&tn) else {
            return Ok(None);
        };
        let tn = reference.name;
        // Builtin static methods, e.g. `Bool.random()`. A user type shadowing a
        // builtin name (`struct Bool { … }`) wins, so only fall back to the
        // builtin when no user type matches.
        let user_defined = reference.user_defined;
        if !user_defined {
            if let Some(recv) = BuiltinReceiver::from_type_name(&tn) {
                if let Some(func) = self.builtins.static_method(recv, method) {
                    let labeled: Vec<Arg> = self
                        .eval_args(arg_nodes)?
                        .into_iter()
                        .map(Arg::from)
                        .collect();
                    return func(self, labeled)
                        .map(Some)
                        .map_err(Self::std_error_to_signal);
                }
            }
        }
        // `Outer.Nested(args)`: construct a nested type referenced through its
        // enclosing type. Nested types are registered by their simple name, so
        // resolve `method` against the type tables when `tn` is itself a user
        // type.
        if user_defined {
            if self.types.is_class(method) {
                let args = self.eval_args(arg_nodes)?;
                return self.instantiate_class(method, args).map(Some);
            }
            if self.types.is_struct(method) {
                let simple: Vec<(Option<String>, SwiftValue)> = self
                    .eval_args(arg_nodes)?
                    .iter()
                    .map(|a| (a.label.clone(), a.value.clone()))
                    .collect();
                return self.instantiate_struct(method, &simple).map(Some);
            }
        }
        if self.enum_has_case(&tn, method) {
            let args = self.eval_args(arg_nodes)?;
            let payload = args.into_iter().map(|a| a.value).collect();
            return Ok(Some(self.make_enum_case(&tn, method, payload)?.unwrap()));
        }
        if self.types.is_struct(&tn) {
            let params = self.user_method_params(&tn, method);
            let args = self.eval_args_with(arg_nodes, params.as_deref())?;
            return self
                .call_struct_method(SwiftValue::Void, &tn, method, args, None)
                .map(Some);
        }
        // `Type.method(...)` — a static method on a class.
        if self.types.is_class(&tn) && self.lookup_method(&tn, method).is_some() {
            let params = self.user_method_params(&tn, method);
            let args = self.eval_args_with(arg_nodes, params.as_deref())?;
            return self
                .dispatch_class_method(SwiftValue::Void, &tn, method, args)
                .map(Some);
        }
        Ok(None)
    }

    fn eval_method_call(&mut self, member: &Node<'static>, arg_nodes: &[Node<'static>]) -> Eval {
        let mut method = member
            .text()
            .ok_or_else(|| EvalError::Unsupported("method without a name".into()))?;
        // A bare `.` member spells its name in the operator slot.
        if method == "." {
            method = member.op_text().unwrap_or(method);
        }

        // Shorthand `.case(args)`: resolve the enum type from msf's inference.
        let Some(base) = member.first_child() else {
            if let Some(tn) = self.resolve_member_enum(member, &method) {
                let args = self.eval_args(arg_nodes)?;
                let payload = args.into_iter().map(|a| a.value).collect();
                return Ok(self.make_enum_case(&tn, &method, payload)?.unwrap());
            }
            // Implicit member static method: `.custom(x)` where the contextual
            // type declares `static func custom`.
            if let Some(tn) = self.resolve_implicit_static_method(member, &method) {
                let params = self.user_method_params(&tn, &method);
                let args = self.eval_args_with(arg_nodes, params.as_deref())?;
                if self.types.is_class(&tn) {
                    return self.dispatch_class_method(SwiftValue::Void, &tn, &method, args);
                }
                return self.call_struct_method(SwiftValue::Void, &tn, &method, args, None);
            }
            return Err(EvalError::Unsupported(format!(".{method}() (unresolved type)")).into());
        };

        // `super.method(args)`: dispatch to the superclass implementation.
        if base.kind() == NodeKind::IdentExpr && base.text().as_deref() == Some("super") {
            let this = self
                .env
                .get("self")
                .ok_or_else(|| EvalError::Unsupported("`super` outside a method".into()))?;
            let start = self
                .class_ctx
                .last()
                .and_then(|c| self.types.class_def(c))
                .and_then(|d| d.superclass.clone())
                .ok_or_else(|| EvalError::Unsupported("`super` without a superclass".into()))?;
            let args = self.eval_args(arg_nodes)?;
            if method == "init" {
                self.run_class_init(this, &start, args)?;
                return Ok(SwiftValue::Void);
            }
            return self.dispatch_class_method(this, &start, &method, args);
        }

        // `self.init(args)`: initializer delegation — a convenience init
        // delegating across to a designated/sibling initializer (classes), or
        // an init delegating to another overload (structs, where it rebuilds
        // the value and rebinds `self`).
        if base.kind() == NodeKind::IdentExpr
            && base.text().as_deref() == Some("self")
            && method == "init"
        {
            // Delegation is only legal while an initializer body is running;
            // Swift rejects `self.init` in ordinary methods at compile time.
            if self.init_ctx == 0 {
                return Err(EvalError::Unsupported(
                    "`self.init` can only be used inside an initializer".into(),
                )
                .into());
            }
            if let Some(this) = self.env.get("self") {
                match &this {
                    SwiftValue::Object(obj) => {
                        let cls = obj.borrow().class_name.clone();
                        let args = self.eval_args(arg_nodes)?;
                        self.run_class_init(this.clone(), &cls, args)?;
                        return Ok(SwiftValue::Void);
                    }
                    SwiftValue::Struct(s) => {
                        let tn = s.type_name.clone();
                        // Integer generic parameter values live as fields on
                        // the instance being initialized; delegation rebuilds
                        // with the same specialization.
                        let type_values: Vec<(String, SwiftValue)> = self
                            .types
                            .struct_def(&tn)
                            .map(|d| d.value_generic_params.clone())
                            .unwrap_or_default()
                            .into_iter()
                            .filter_map(|n| s.get(&n).cloned().map(|v| (n, v)))
                            .collect();
                        let args = self.eval_args(arg_nodes)?;
                        let simple: Vec<(Option<String>, SwiftValue)> = args
                            .iter()
                            .map(|a| (a.label.clone(), a.value.clone()))
                            .collect();
                        let rebuilt =
                            self.instantiate_struct_specialized(&tn, &simple, &type_values)?;
                        // A failed failable delegate (`self.init?(...)` returned
                        // nil) fails the delegating initializer too.
                        if matches!(rebuilt, SwiftValue::Nil) {
                            return Err(Signal::Return(SwiftValue::Nil));
                        }
                        if self.env.assign("self", rebuilt).is_err() {
                            return Err(EvalError::Unsupported(
                                "`self.init` outside an initializer".into(),
                            )
                            .into());
                        }
                        return Ok(SwiftValue::Void);
                    }
                    _ => {}
                }
            }
        }

        // `Task.*` concurrency entry points (ADR-0005).
        if let Some(result) = self.try_task_type_method(&base, &method, arg_nodes)? {
            return Ok(result);
        }

        // `MainActor.run { }` hop (ADR-0005).
        if let Some(result) = self.try_main_actor_method(&base, &method, arg_nodes)? {
            return Ok(result);
        }

        // `AsyncStream.makeStream(of:)` factory (ADR-0005).
        if let Some(result) = self.try_async_stream_static(&base, &method, arg_nodes)? {
            return Ok(result);
        }

        // `Type.<...>(args)`: builtin static method, nested-type construction,
        // enum case, or a static struct/class method — all resolved off the
        // type named by `base`.
        if let Some(v) = self.try_type_qualified_method(&base, &method, arg_nodes)? {
            return Ok(v);
        }

        let base_value = self.eval(&base)?;

        // An optional-chained method call on an absent base (`none?.f()`)
        // nil-propagates. Type-qualified and implicit-member bases were
        // handled above, so a Nil here is a real absent value.
        if matches!(base_value, SwiftValue::Nil) {
            return Ok(SwiftValue::Nil);
        }

        // `group.addTask { }` / `group.cancelAll()` and `task.cancel()`.
        if let Some(result) = self.try_concurrency_method(&base_value, &method, arg_nodes)? {
            return Ok(result);
        }

        // `JSONEncoder().encode(...)` / `JSONDecoder().decode(...)` (Codable).
        if let Some(v) = self.try_json_coder_method(&base_value, &method, arg_nodes)? {
            return Ok(v);
        }
        // `PropertyListEncoder().encode(...)` — XML plist serialiser.
        if let Some(v) = self.try_plist_coder_method(&base_value, &method, arg_nodes)? {
            return Ok(v);
        }

        // Class instance: dynamic dispatch from the runtime class.
        if let Some(v) = self.try_class_instance_method(&base_value, &method, arg_nodes)? {
            return Ok(v);
        }

        // User extension method on a builtin type (`extension Int { … }`),
        // consulted before the stdlib seam so a program's extension can shadow
        // an otherwise-available intrinsic/algorithm.
        if let Some(v) = self.try_builtin_ext_method(&base_value, &base, &method, arg_nodes)? {
            return Ok(v);
        }

        // Builtin-receiver method layer: label-aware overloads, type-specific
        // intrinsics, then `Sequence`/`Collection` algorithms — all keyed off the
        // receiver's [`BuiltinReceiver`] kind, resolved in one place.
        if let Some(v) = self.try_builtin_receiver_method(&base_value, &base, &method, arg_nodes)? {
            return Ok(v);
        }

        // `Result.get()`: unwrap success, or throw the failure error.
        if let Some(v) = self.try_result_get(&base_value, &method)? {
            return Ok(v);
        }

        // AsyncSequence algorithms (`reduce`/`map`/`filter`/`contains`/…): the
        // executor runs the producer to completion (ADR-0005), so materialise
        // the elements and reuse the eager array algorithm machinery.
        if let Some(v) = self.try_async_sequence_method(&base_value, &base, &method, arg_nodes)? {
            return Ok(v);
        }

        // User struct/enum method, then the generic struct-method fallback
        // (SwiftUI view modifiers) dispatched on any struct receiver by name.
        // `base_value` is moved in (not cloned) so a mutating method sees a
        // uniquely-referenced receiver — the copy-on-write / `make_mut` and
        // `isKnownUniquelyReferenced` semantics depend on the strong count. The
        // error name is captured first since the value is consumed on a miss.
        let builtin_name = base_value.type_name();
        if let Some(v) = self.try_struct_receiver_method(base_value, &base, &method, arg_nodes)? {
            return Ok(v);
        }

        Err(EvalError::Unsupported(format!("method .{method}() on {builtin_name}")).into())
    }

    /// Dispatch a method on a class instance (`SwiftValue::Object`) dynamically
    /// from its runtime class. Returns `Ok(None)` when `base_value` is not an
    /// object.
    fn try_class_instance_method(
        &mut self,
        base_value: &SwiftValue,
        method: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        if let SwiftValue::Object(obj) = base_value {
            let class_name = obj.borrow().class_name.clone();
            let params = self.user_method_params(&class_name, method);
            let args = self.eval_args_with(arg_nodes, params.as_deref())?;
            return self
                .dispatch_class_method(base_value.clone(), &class_name, method, args)
                .map(Some);
        }
        Ok(None)
    }

    /// Dispatch a user extension method declared on a builtin type
    /// (`extension Int { … }`). Returns `Ok(None)` when no such extension member
    /// matches so the stdlib seam is consulted next.
    fn try_builtin_ext_method(
        &mut self,
        base_value: &SwiftValue,
        base: &Node<'static>,
        method: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        let builtin_name = base_value.type_name();
        if self.types.has_builtin_ext_method(&builtin_name, method) {
            let params = self
                .types
                .builtin_ext_method(&builtin_name, method)
                .map(|def| clone_params(&def.params));
            let args = self.eval_args_with(arg_nodes, params.as_deref())?;
            let place = self.resolve_place(base);
            if let Some(result) =
                self.call_builtin_ext_method(base_value.clone(), &builtin_name, method, args, place)
            {
                return result.map(Some);
            }
        }
        Ok(None)
    }

    /// `Result.get()`: unwrap a `.success` payload, or throw the `.failure`
    /// error. Returns `Ok(None)` when `base_value` is not a `Result` `.get()`.
    fn try_result_get(
        &self,
        base_value: &SwiftValue,
        method: &str,
    ) -> Result<Option<SwiftValue>, Signal> {
        if let SwiftValue::Enum(e) = base_value {
            if e.type_name == "Result" && method == "get" {
                return match e.case.as_str() {
                    "success" => Ok(Some(e.payload.first().cloned().unwrap_or(SwiftValue::Void))),
                    _ => Err(Signal::Throw(
                        e.payload.first().cloned().unwrap_or(SwiftValue::Nil),
                    )),
                };
            }
        }
        Ok(None)
    }

    /// Dispatch a method on a struct/enum receiver: a user-declared method
    /// first, then the generic struct-method fallback (the SwiftUI view-modifier
    /// seam) dispatched on any struct receiver by name. Returns `Ok(None)` when
    /// neither matches, leaving the caller to report an unsupported method.
    fn try_struct_receiver_method(
        &mut self,
        base_value: SwiftValue,
        base: &Node<'static>,
        method: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        let type_name = match &base_value {
            SwiftValue::Struct(o) => Some(o.type_name.clone()),
            SwiftValue::Enum(e) => Some(e.type_name.clone()),
            _ => None,
        };
        let method_params = type_name
            .as_ref()
            .and_then(|tn| self.user_method_params(tn, method))
            .or_else(|| {
                self.globals
                    .struct_method(method)
                    .and_then(|e| e.params.as_ref())
                    .map(|p| clone_params(p))
            });
        let args = self.eval_args_with(arg_nodes, method_params.as_deref())?;
        if let Some(type_name) = type_name {
            if self.type_has_method(&type_name, method) {
                let place = self.resolve_place(base);
                return self
                    .call_struct_method(base_value, &type_name, method, args, place)
                    .map(Some);
            }
        }

        // Generic struct-method fallback (SwiftUI view modifiers): dispatched on
        // any struct receiver by name, after user methods and builtin receivers.
        if matches!(base_value, SwiftValue::Struct(_)) {
            if let Some(func) = self.globals.struct_method(method).map(|e| e.f) {
                let labeled: Vec<Arg> = args.into_iter().map(Arg::from).collect();
                return func(self, base_value, labeled)
                    .map(Some)
                    .map_err(Self::std_error_to_signal);
            }
        }

        Ok(None)
    }

    /// Run the initializer declared at or above `start_class` for `this`,
    /// selecting among overloads by the call's argument labels. Used for both
    /// `super.init(...)` chaining and `self.init(...)` delegation.
    fn run_class_init(
        &mut self,
        this: SwiftValue,
        start_class: &str,
        args: Vec<CallArg>,
    ) -> Result<(), Signal> {
        let mut chain = self.class_chain(start_class);
        chain.reverse(); // most-derived (start) first
                         // Prefer the closest class whose declared initializers label-match the
                         // call — this also finds an *inherited* initializer when the subclass
                         // declares its own with different labels.
        let selected = chain.iter().find_map(|c| {
            let def = self.types.class_def(c)?;
            select_labeled_overload(&def.init_overloads, &args)
                .map(|m| (c.clone(), clone_params(&m.params), m.body))
        });
        let (owner, params, body) = match selected {
            Some(t) => t,
            None => {
                let Some(owner) = chain
                    .into_iter()
                    .find(|c| self.types.class_def(c).unwrap().init.is_some())
                else {
                    return Ok(()); // no explicit init to run
                };
                let m = self.types.class_def(&owner).unwrap().init.as_ref().unwrap();
                (owner, clone_params(&m.params), m.body)
            }
        };
        // Depth-guarded: `self.init` delegation re-enters here, and a
        // self-recursive delegate must trap instead of overflowing the native
        // stack.
        self.depth += 1;
        if self.depth > MAX_CALL_DEPTH {
            self.depth -= 1;
            return Err(trap(
                "stack overflow: initializer delegation too deep".into(),
            ));
        }
        self.class_ctx.push(owner);
        let saved_env = self.env.enter_isolated();
        self.env.declare("self", this, false);
        let bound = self.bind_params(&params, args);
        self.init_ctx += 1;
        let result = match bound {
            Ok(_) => match body {
                Some(b) => self.eval(&b),
                None => Ok(SwiftValue::Void),
            },
            Err(e) => Err(e),
        };
        self.init_ctx -= 1;
        self.env.restore(saved_env);
        self.class_ctx.pop();
        self.depth -= 1;
        match result {
            // A failing `super.init?` (`return nil`) must propagate so the
            // calling subclass initializer also fails, rather than producing a
            // half-built instance.
            Err(Signal::Return(SwiftValue::Nil)) => Err(Signal::Return(SwiftValue::Nil)),
            Ok(_) | Err(Signal::Return(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Dispatch a class method dynamically (override-aware), binding `self`.
    pub(super) fn dispatch_class_method(
        &mut self,
        this: SwiftValue,
        from_class: &str,
        method: &str,
        args: Vec<CallArg>,
    ) -> Eval {
        let (params, body, owner, generics) = match self.lookup_method(from_class, method) {
            Some(m) => m,
            None => match self.protocol_default_method(from_class, method) {
                Some((p, b, _, g)) => (p, b, from_class.to_string(), g),
                // An `@objc optional` method requirement the conformer does
                // not implement resolves to nil — for chained and plain calls
                // alike (the parser drops the `?`; documented permissiveness).
                None if self.protocol_optional_method(from_class, method) => {
                    return Ok(SwiftValue::Nil);
                }
                None => {
                    return Err(EvalError::Unsupported(format!(
                        "{from_class} has no method `{method}`"
                    ))
                    .into());
                }
            },
        };
        // A type-level (`static`/`class`) method has no instance `self`.
        let is_static_call = matches!(this, SwiftValue::Void);
        if is_static_call {
            self.static_ctx.push(from_class.to_string());
        }
        self.class_ctx.push(owner);
        let type_binding = self.infer_type_bindings(&generics, &params, &args);
        self.type_bindings.push(type_binding);
        // Isolate from caller locals (a class `self` is a reference, so field
        // mutations persist through the object regardless of the env).
        let saved_env = self.env.enter_isolated();
        self.env.declare("self", this, false);
        let bound = self.bind_params(&params, args);
        let result = match bound {
            Ok(_) => match body {
                Some(b) => self.eval(&b),
                None => Ok(SwiftValue::Void),
            },
            Err(e) => Err(e),
        };
        self.env.restore(saved_env);
        self.type_bindings.pop();
        self.class_ctx.pop();
        if is_static_call {
            self.static_ctx.pop();
        }
        match result {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    /// Select a struct method overload by the call's argument labels. Returns
    /// `None` unless the type declares more than one method of that name and
    /// exactly one of them matches the labels — keeping single-method dispatch
    /// and unresolved (type-only) overloads on the existing path.
    fn select_struct_overload(
        &self,
        type_name: &str,
        method: &str,
        args: &[CallArg],
    ) -> Option<(Vec<Param>, Option<Node<'static>>, bool, Vec<String>)> {
        let overloads = self
            .types
            .struct_def(type_name)?
            .method_overloads
            .get(method)?;
        if overloads.len() < 2 {
            return None;
        }
        let chosen = select_labeled_overload(overloads, args)?;
        Some((
            clone_params(&chosen.params),
            chosen.body,
            chosen.mutating,
            chosen.generic_params.clone(),
        ))
    }

    /// The declared parameters of a user method on `type_name`, across class,
    /// struct, and enum types (used to spot `@autoclosure` params before the
    /// arguments are evaluated).
    fn user_method_params(&self, type_name: &str, method: &str) -> Option<Vec<Param>> {
        if let Some((params, _, _, _)) = self.lookup_method(type_name, method) {
            return Some(params);
        }
        if let Some(d) = self.types.struct_def(type_name) {
            if let Some(m) = d.methods.get(method) {
                return Some(clone_params(&m.params));
            }
        }
        if let Some(d) = self.types.enum_def(type_name) {
            if let Some(m) = d.methods.get(method) {
                return Some(clone_params(&m.params));
            }
        }
        None
    }

    /// Whether a struct or enum type declares a method `method`.
    pub(super) fn type_has_method(&self, type_name: &str, method: &str) -> bool {
        self.types
            .struct_def(type_name)
            .is_some_and(|d| d.methods.contains_key(method))
            || self
                .types
                .enum_def(type_name)
                .is_some_and(|d| d.methods.contains_key(method))
            || self.protocol_default_method(type_name, method).is_some()
    }

    /// Invoke a struct method with `self` bound and parameters applied.
    pub(super) fn call_struct_method(
        &mut self,
        this: SwiftValue,
        type_name: &str,
        method: &str,
        args: Vec<CallArg>,
        base_place: Option<Place>,
    ) -> Eval {
        // Prefer a label-selected overload (`buildEither(first:)` vs
        // `(second:)`); fall back to the single stored method otherwise.
        let own = self
            .select_struct_overload(type_name, method, &args)
            .or_else(|| {
                self.types
                    .struct_def(type_name)
                    .and_then(|d| d.methods.get(method))
                    .or_else(|| {
                        self.types
                            .enum_def(type_name)
                            .and_then(|d| d.methods.get(method))
                    })
                    .map(|def| {
                        (
                            clone_params(&def.params),
                            def.body,
                            def.mutating,
                            def.generic_params.clone(),
                        )
                    })
            });
        let (params, body, mutating, generics) = match own {
            Some(m) => m,
            None => self
                .protocol_default_method(type_name, method)
                .ok_or_else(|| {
                    EvalError::Unsupported(format!("{type_name} has no method `{method}`"))
                })?,
        };

        // A `static`/type method has no instance `self`; record the type so an
        // unqualified static-property reference inside it resolves.
        let is_static_call = matches!(this, SwiftValue::Void);
        if is_static_call {
            self.static_ctx.push(type_name.to_string());
        }
        let type_binding = self.infer_type_bindings(&generics, &params, &args);
        self.type_bindings.push(type_binding);
        // For a `mutating` struct method with an lvalue receiver, take the
        // receiver out of its storage so `self` becomes its sole owner *aside
        // from other logical bindings* (the `var y = x` aliases). `make_mut`
        // then clones the `StructObj` — retaining its reference-type fields —
        // exactly when the value is shared, so a class-backed CoW buffer reads
        // the right answer from `isKnownUniquelyReferenced`. A unique value
        // keeps strong count 1 and is mutated in place. The end-of-call
        // write-back restores the storage we vacated here.
        let this = if mutating && matches!(this, SwiftValue::Struct(_)) {
            // Only a *root* stored binding is vacated: vacating a nested member
            // (`outer.buffer.append(...)`) would route the placeholder write
            // through `willSet`/`didSet`/computed setters/property wrappers,
            // which must not observe the transient. Nested receivers — and
            // roots that are themselves accessor-backed variables (computed or
            // observed globals/locals) — keep the pre-existing
            // clone-and-write-back behaviour.
            match &base_place {
                Some(place)
                    if place.path.is_empty()
                        && !matches!(
                            self.env.get(&place.root),
                            Some(SwiftValue::AccessorVar(_))
                        ) =>
                {
                    drop(this);
                    let mut taken = self.read_place(place)?;
                    self.write_place(place, SwiftValue::Void)?;
                    if let SwiftValue::Struct(rc) = &mut taken {
                        let _ = Rc::make_mut(rc);
                    }
                    taken
                }
                _ => this,
            }
        } else {
            this
        };
        // Run isolated from the caller's locals: the body sees globals, its
        // parameters, and `self`/its members, but not enclosing variables.
        let saved_env = self.env.enter_isolated();
        self.env.declare("self", this, true);
        let (outcome, inout_finals) = match self.bind_params(&params, args) {
            Ok(binds) => {
                let result = match body {
                    Some(b) => self.eval(&b),
                    None => Ok(SwiftValue::Void),
                };
                // Capture `inout` write-backs against the method env before it
                // is torn down; apply them to the caller below.
                let finals: Vec<(Place, SwiftValue)> = binds
                    .iter()
                    .filter_map(|(name, place)| self.env.get(name).map(|v| (place.clone(), v)))
                    .collect();
                (result, finals)
            }
            Err(e) => (Err(e), Vec::new()),
        };
        let updated_self = self.env.get("self").unwrap_or(SwiftValue::Void);
        self.env.restore(saved_env);
        self.type_bindings.pop();
        if is_static_call {
            self.static_ctx.pop();
        }

        // Write `inout` parameters and the mutated receiver back to the caller,
        // including on a thrown error (Swift copies them out on a caught
        // error); only a fatal interpreter trap skips the copy-out.
        if !matches!(outcome, Err(Signal::Error(_))) {
            for (place, v) in inout_finals {
                self.write_place(&place, v)?;
            }
            if mutating {
                if let Some(place) = base_place {
                    self.write_place(&place, updated_self)?;
                }
            }
        }
        match outcome {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    /// Run a user extension method declared on a builtin type, binding `self`
    /// to the receiver and writing it back through `place` for a `mutating`
    /// method.
    fn call_builtin_ext_method(
        &mut self,
        receiver: SwiftValue,
        type_name: &str,
        method: &str,
        args: Vec<CallArg>,
        place: Option<Place>,
    ) -> Option<Eval> {
        let def = self.types.builtin_ext_method(type_name, method)?;
        let params = clone_params(&def.params);
        let body = def.body;
        let mutating = def.mutating;
        if mutating && place.is_none() {
            return Some(Err(EvalError::Type(format!(
                "mutating method `{method}` requires an lvalue receiver"
            ))
            .into()));
        }
        let saved_env = self.env.enter_isolated();
        self.env.declare("self", receiver, true);
        let outcome = match self.bind_params(&params, args) {
            Ok(_) => match body {
                Some(b) => self.eval(&b),
                None => Ok(SwiftValue::Void),
            },
            Err(e) => Err(e),
        };
        let updated_self = self.env.get("self").unwrap_or(SwiftValue::Void);
        self.env.restore(saved_env);
        let result = match outcome {
            Ok(v) => Ok(v),
            Err(Signal::Return(v)) => Ok(v),
            Err(e) => Err(e),
        };
        // A `mutating` method copies the updated receiver back, including when
        // it throws (Swift writes `inout self` back on a caught error); only a
        // fatal interpreter trap skips the copy-out.
        if mutating && !matches!(result, Err(Signal::Error(_))) {
            if let Some(place) = place {
                if let Err(e) = self.write_place(&place, updated_self) {
                    return Some(Err(e));
                }
            }
        }
        Some(result)
    }
}

/// Parse an integer generic argument (`Buf<4>`, `Buf<0xFF>`) in any Swift
/// radix, honouring `_` separators.
fn parse_int_generic_arg(text: &str) -> Option<i128> {
    let s = text.replace('_', "");
    let (digits, radix) = if let Some(rest) = s.strip_prefix("0x") {
        (rest, 16)
    } else if let Some(rest) = s.strip_prefix("0b") {
        (rest, 2)
    } else if let Some(rest) = s.strip_prefix("0o") {
        (rest, 8)
    } else {
        (s.as_str(), 10)
    };
    i128::from_str_radix(digits, radix).ok()
}
