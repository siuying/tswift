use std::rc::Rc;

use tswift_frontend::{Node, NodeKind};

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
        let entry = *self.labeled_intrinsics.get(&(kind, method.to_string()))?;
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
            let def = self.classes.get(&cls)?;
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
                        self.free_fns
                            .get(&name)
                            .and_then(|e| e.params.as_ref())
                            .map(|p| clone_params(p))
                    }
                }
            })
        } else {
            None
        };
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
            if self.enums.contains_key(&name) {
                if let Some(raw) = args
                    .iter()
                    .find(|a| a.label.as_deref() == Some("rawValue"))
                    .map(|a| a.value.clone())
                {
                    let case = self.enums[&name]
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
            if self.classes.contains_key(&name) {
                return self.instantiate_class(&name, args);
            }
            // Struct memberwise initializer.
            if self.structs.contains_key(&name) {
                let simple: Vec<(Option<String>, SwiftValue)> = args
                    .iter()
                    .map(|a| (a.label.clone(), a.value.clone()))
                    .collect();
                return self.instantiate_struct(&name, &simple);
            }
            // `@dynamicCallable`: calling a struct instance routes through its
            // `dynamicallyCall(...)` method.
            if let Some(value @ SwiftValue::Struct(_)) = self.env.get(&name) {
                if self.is_dynamic_callable(&value) {
                    return self.dynamic_call(value, args);
                }
            }
            // A bound function or closure value (incl. recursion).
            match self.env.get(&name) {
                Some(SwiftValue::Function(id)) => return self.call_function(id, args),
                Some(SwiftValue::Closure(id)) => {
                    return self.call_closure_with_args(id, args);
                }
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
                if let Some(ctor) = self.builtin_ctors.get(name.as_str()).copied() {
                    if let Some(v) = ctor(self, &name, &args)? {
                        return Ok(v);
                    }
                }
            }

            // Free-function intrinsic served through the StdContext seam.
            if let Some(free) = self.free_fns.get(&name).map(|e| e.f) {
                let labeled: Vec<Arg> = args.into_iter().map(Arg::from).collect();
                return free(self, labeled).map_err(Self::std_error_to_signal);
            }
            if let Some(native) = self.natives.get(&name).copied() {
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
                    let optional = ty.trim().ends_with('?');
                    if !(optional && kind == NodeKind::NilLiteral) {
                        value = self.coerce_literal_value(
                            ty.trim().trim_end_matches('?').trim(),
                            kind,
                            value,
                        )?;
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
                if self.classes.contains_key(&tn) {
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
                .and_then(|c| self.classes.get(c))
                .and_then(|d| d.superclass.clone())
                .ok_or_else(|| EvalError::Unsupported("`super` without a superclass".into()))?;
            let args = self.eval_args(arg_nodes)?;
            if method == "init" {
                self.run_class_init(this, &start, args)?;
                return Ok(SwiftValue::Void);
            }
            return self.dispatch_class_method(this, &start, &method, args);
        }

        // `Task.*` concurrency entry points (ADR-0005).
        if let Some(result) = self.try_task_type_method(&base, &method, arg_nodes)? {
            return Ok(result);
        }

        // `Type.<...>(args)`: enum case construction or a static struct method.
        if base.kind() == NodeKind::IdentExpr {
            if let Some(tn) = base.text() {
                // `Self.method(...)` calls a static method of the enclosing type
                // (the keyword is never a value binding); any other name is a
                // type reference only when no local value shadows it.
                if let Some(reference) = self.resolve_type_reference(&tn) {
                    let tn = reference.name;
                    // Builtin static methods, e.g. `Bool.random()`. A user type
                    // shadowing a builtin name (`struct Bool { … }`) wins, so
                    // only fall back to the builtin when no user type matches.
                    let user_defined = reference.user_defined;
                    if !user_defined {
                        if let Some(recv) = BuiltinReceiver::from_type_name(&tn) {
                            if let Some(func) =
                                self.static_methods.get(&(recv, method.clone())).copied()
                            {
                                let labeled: Vec<Arg> = self
                                    .eval_args(arg_nodes)?
                                    .into_iter()
                                    .map(Arg::from)
                                    .collect();
                                return func(self, labeled).map_err(Self::std_error_to_signal);
                            }
                        }
                    }
                    // `Outer.Nested(args)`: construct a nested type referenced
                    // through its enclosing type. Nested types are registered by
                    // their simple name, so resolve `method` against the type
                    // tables when `tn` is itself a user type.
                    if user_defined {
                        if self.classes.contains_key(&method) {
                            let args = self.eval_args(arg_nodes)?;
                            return self.instantiate_class(&method, args);
                        }
                        if self.structs.contains_key(&method) {
                            let simple: Vec<(Option<String>, SwiftValue)> = self
                                .eval_args(arg_nodes)?
                                .iter()
                                .map(|a| (a.label.clone(), a.value.clone()))
                                .collect();
                            return self.instantiate_struct(&method, &simple);
                        }
                    }
                    if self.enum_has_case(&tn, &method) {
                        let args = self.eval_args(arg_nodes)?;
                        let payload = args.into_iter().map(|a| a.value).collect();
                        return Ok(self.make_enum_case(&tn, &method, payload)?.unwrap());
                    }
                    if self.structs.contains_key(&tn) {
                        let params = self.user_method_params(&tn, &method);
                        let args = self.eval_args_with(arg_nodes, params.as_deref())?;
                        return self.call_struct_method(SwiftValue::Void, &tn, &method, args, None);
                    }
                    // `Type.method(...)` — a static method on a class.
                    if self.classes.contains_key(&tn) && self.lookup_method(&tn, &method).is_some()
                    {
                        let params = self.user_method_params(&tn, &method);
                        let args = self.eval_args_with(arg_nodes, params.as_deref())?;
                        return self.dispatch_class_method(SwiftValue::Void, &tn, &method, args);
                    }
                }
            }
        }

        let base_value = self.eval(&base)?;

        // `group.addTask { }` / `group.cancelAll()` and `task.cancel()`.
        if let Some(result) = self.try_concurrency_method(&base_value, &method, arg_nodes)? {
            return Ok(result);
        }

        // `JSONEncoder().encode(...)` / `JSONDecoder().decode(...)` (Codable).
        if let Some(v) = self.try_json_coder_method(&base_value, &method, arg_nodes)? {
            return Ok(v);
        }

        // Class instance: dynamic dispatch from the runtime class.
        if let SwiftValue::Object(obj) = &base_value {
            let class_name = obj.borrow().class_name.clone();
            let params = self.user_method_params(&class_name, &method);
            let args = self.eval_args_with(arg_nodes, params.as_deref())?;
            return self.dispatch_class_method(base_value.clone(), &class_name, &method, args);
        }

        // User extension method on a builtin type (`extension Int { … }`).
        // User declarations are consulted before the stdlib seam so a program's
        // extension can shadow an otherwise-available intrinsic/algorithm.
        let builtin_name = base_value.type_name();
        if self
            .builtin_ext_methods
            .get(&builtin_name)
            .is_some_and(|m| m.contains_key(&method))
        {
            let params = self
                .builtin_ext_methods
                .get(&builtin_name)
                .and_then(|m| m.get(&method))
                .map(|def| clone_params(&def.params));
            let args = self.eval_args_with(arg_nodes, params.as_deref())?;
            let place = self.resolve_place(&base);
            if let Some(result) = self.call_builtin_ext_method(
                base_value.clone(),
                &builtin_name,
                &method,
                args,
                place,
            ) {
                return result;
            }
        }

        let mut evaluated_args = None;

        // Label-aware stdlib overloads (layer 1a): selected APIs need argument
        // labels to choose between overloads without leaking that policy into the
        // interpreter dispatcher.
        if let Some(kind) = BuiltinReceiver::of(&base_value) {
            if self
                .labeled_intrinsics
                .contains_key(&(kind, method.clone()))
            {
                let args = self.eval_args(arg_nodes)?;
                let labeled: Vec<Arg> = args.iter().map(Arg::from).collect();
                let place = self.resolve_place(&base);
                if let Some(result) =
                    self.dispatch_labeled_intrinsic(base_value.clone(), &method, labeled, place)
                {
                    return result;
                }
                evaluated_args = Some(args);
            }
        }

        // Standard-library intrinsic registry (layer 1): type-specific members
        // such as `Array.append`. Consulted before the ad-hoc algorithm paths.
        if let Some(kind) = BuiltinReceiver::of(&base_value) {
            if let Some(entry) = self.intrinsics.get(&(kind, method.clone())).copied() {
                let args = match evaluated_args.take() {
                    Some(args) => args,
                    None => self.eval_args(arg_nodes)?,
                };
                // `IndexPath`/`IndexSet` intrinsics take positional arguments.
                // The sole exception is `IndexSet.update(with:)`, whose one
                // argument is labelled `with:` (and requires that label).
                if matches!(kind, BuiltinReceiver::IndexPath | BuiltinReceiver::IndexSet) {
                    let is_update = kind == BuiltinReceiver::IndexSet && method == "update";
                    let labels_valid = args.iter().all(|arg| match arg.label.as_deref() {
                        Some("with") => is_update,
                        Some(_) => false,
                        None => !is_update,
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
                let place = self.resolve_place(&base);
                return match (entry.func)(self, base_value, plain) {
                    Ok(outcome) => self.apply_method_outcome(outcome, entry.mutating, place),
                    Err(err) => Err(Self::std_error_to_signal(err)),
                };
            }
        }

        // Standard-library algorithm layer (layer 2): `Sequence`/`Collection`
        // methods (`map`/`filter`/`sorted`/…) over any builtin sequence.
        if self.algorithms.contains_key(&method) {
            let items = if let Some(items) = materialize_sequence(&base_value) {
                Some(items)
            } else if self.is_custom_sequence(&base_value) {
                Some(self.materialize_custom_sequence(base_value.clone())?)
            } else {
                None
            };
            if let Some(items) = items {
                let func = self.algorithms[&method];
                let labeled: Vec<Arg> = self
                    .eval_args(arg_nodes)?
                    .into_iter()
                    .map(Arg::from)
                    .collect();
                return func(self, items, labeled).map_err(Self::std_error_to_signal);
            }
        }

        // `Result.get()`: unwrap success, or throw the failure error.
        if let SwiftValue::Enum(e) = &base_value {
            if e.type_name == "Result" && method == "get" {
                return match e.case.as_str() {
                    "success" => Ok(e.payload.first().cloned().unwrap_or(SwiftValue::Void)),
                    _ => Err(Signal::Throw(
                        e.payload.first().cloned().unwrap_or(SwiftValue::Nil),
                    )),
                };
            }
        }

        let type_name = match &base_value {
            SwiftValue::Struct(o) => Some(o.type_name.clone()),
            SwiftValue::Enum(e) => Some(e.type_name.clone()),
            _ => None,
        };
        let method_params = type_name
            .as_ref()
            .and_then(|tn| self.user_method_params(tn, &method))
            .or_else(|| {
                self.struct_methods
                    .get(&method)
                    .and_then(|e| e.params.as_ref())
                    .map(|p| clone_params(p))
            });
        let args = self.eval_args_with(arg_nodes, method_params.as_deref())?;
        if let Some(type_name) = type_name {
            if self.type_has_method(&type_name, &method) {
                let place = self.resolve_place(&base);
                return self.call_struct_method(base_value, &type_name, &method, args, place);
            }
        }

        // Generic struct-method fallback (SwiftUI view modifiers): dispatched on
        // any struct receiver by name, after user methods and builtin receivers.
        if matches!(base_value, SwiftValue::Struct(_)) {
            if let Some(func) = self.struct_methods.get(&method).map(|e| e.f) {
                let labeled: Vec<Arg> = args.into_iter().map(Arg::from).collect();
                return func(self, base_value, labeled).map_err(Self::std_error_to_signal);
            }
        }

        let builtin_name = base_value.type_name();
        Err(EvalError::Unsupported(format!("method .{method}() on {builtin_name}")).into())
    }

    /// Run the initializer declared at or above `start_class` for `this`.
    fn run_class_init(
        &mut self,
        this: SwiftValue,
        start_class: &str,
        args: Vec<CallArg>,
    ) -> Result<(), Signal> {
        let mut chain = self.class_chain(start_class);
        chain.reverse(); // most-derived (start) first
        let owner = chain.into_iter().find(|c| self.classes[c].init.is_some());
        let Some(owner) = owner else {
            return Ok(()); // no explicit init to run
        };
        let (params, body) = {
            let m = self.classes[&owner].init.as_ref().unwrap();
            (clone_params(&m.params), m.body)
        };
        self.class_ctx.push(owner);
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
        self.class_ctx.pop();
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
            None => {
                let (p, b, _, g) = self
                    .protocol_default_method(from_class, method)
                    .ok_or_else(|| {
                        EvalError::Unsupported(format!("{from_class} has no method `{method}`"))
                    })?;
                (p, b, from_class.to_string(), g)
            }
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
        let overloads = self.structs.get(type_name)?.method_overloads.get(method)?;
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
        if let Some(d) = self.structs.get(type_name) {
            if let Some(m) = d.methods.get(method) {
                return Some(clone_params(&m.params));
            }
        }
        if let Some(d) = self.enums.get(type_name) {
            if let Some(m) = d.methods.get(method) {
                return Some(clone_params(&m.params));
            }
        }
        None
    }

    /// Whether a struct or enum type declares a method `method`.
    pub(super) fn type_has_method(&self, type_name: &str, method: &str) -> bool {
        self.structs
            .get(type_name)
            .is_some_and(|d| d.methods.contains_key(method))
            || self
                .enums
                .get(type_name)
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
                self.structs
                    .get(type_name)
                    .and_then(|d| d.methods.get(method))
                    .or_else(|| {
                        self.enums
                            .get(type_name)
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
            // which must not observe the transient. Nested receivers keep the
            // pre-existing clone-and-write-back behaviour.
            match &base_place {
                Some(place) if place.path.is_empty() => {
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
        let def = self.builtin_ext_methods.get(type_name)?.get(method)?;
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
