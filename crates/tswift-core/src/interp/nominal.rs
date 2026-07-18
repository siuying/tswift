use tswift_frontend::{Node, NodeKind};

use super::{
    clone_method, clone_params, expand_directives, field_type_name, generic_param_names, is_expr,
    is_value_node, parse_params, ClassDef, ComputedProp, EnumCaseDef, EnumDef, Interpreter,
    MethodDef, MethodOverload, RawKind, StoredProp, StructDef, SubscriptDef, WrapperDef,
};
use crate::value::{IntWidth, SwiftValue};

impl<'w> Interpreter<'w> {
    /// Pre-declare function and struct declarations in `node` so forward
    /// references resolve. Also records top-level `import` modules (ADR-0020
    /// Phase C) so they are known before body evaluation.
    pub(super) fn hoist(&mut self, node: &Node<'static>) {
        // First pass: type and protocol declarations + import collection.
        for child in expand_directives(node) {
            match child.kind() {
                NodeKind::FuncDecl => self.declare_func(&child),
                NodeKind::StructDecl => {
                    self.register_struct(&child);
                    self.register_nested_types(&child);
                }
                NodeKind::EnumDecl => {
                    self.register_enum(&child);
                    self.register_nested_types(&child);
                }
                // An `actor` is a reference type whose isolation is provided
                // for free by our single-threaded executor (ADR-0005), so it is
                // registered exactly like a class.
                NodeKind::ClassDecl | NodeKind::ActorDecl => {
                    self.register_class(&child);
                    self.register_nested_types(&child);
                }
                NodeKind::ProtocolDecl => self.register_protocol(&child),
                NodeKind::TypeAliasDecl => self.register_typealias(&child),
                // Phase C: collect imported modules before body evaluation.
                // Lenient — no gating; path leading component only.
                NodeKind::ImportDecl => {
                    if let Some(path) = child.text() {
                        self.mark_module_imported(&path);
                    }
                }
                _ => {}
            }
        }
        // Second pass: extensions (they add to already-registered types).
        for child in expand_directives(node) {
            if child.kind() == NodeKind::ExtensionDecl {
                self.register_extension(&child);
            }
        }
    }

    /// Register type declarations nested inside a type body so they resolve by
    /// their simple name (e.g. `B` referenced inside `A`, or `A.B` qualified).
    fn register_nested_types(&mut self, node: &Node<'static>) {
        // Members are the nominal's direct children; there is no synthesized
        // body block. Non-member children (inherited types, attributes, generic
        // params) fall through each loop's `_ => {}` arm.
        let body = node;
        for member in expand_directives(body) {
            match member.kind() {
                NodeKind::StructDecl => {
                    self.register_struct(&member);
                    self.register_nested_types(&member);
                }
                NodeKind::EnumDecl => {
                    self.register_enum(&member);
                    self.register_nested_types(&member);
                }
                NodeKind::ClassDecl | NodeKind::ActorDecl => {
                    self.register_class(&member);
                    self.register_nested_types(&member);
                }
                _ => {}
            }
        }
    }

    /// Register a `typealias X = A & B` whose right-hand side is a protocol
    /// composition, so conformance to `X` can be expanded to its components.
    fn register_typealias(&mut self, node: &Node<'static>) {
        let Some(name) = node.text() else { return };
        let Some(rhs) = node
            .children()
            .find(|c| c.kind() == NodeKind::TypeRef)
            .and_then(|c| c.text())
        else {
            return;
        };
        if !rhs.contains('&') {
            return;
        }
        let components: Vec<String> = rhs
            .split('&')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !components.is_empty() {
            self.types.add_protocol_alias(name, components);
        }
    }

    /// Record the protocols a type conforms to from its inherited-type
    /// (`TypeRef`) children.
    fn record_conformances(&mut self, type_name: &str, node: &Node<'static>) {
        let conf: Vec<String> = node
            .children()
            .filter(|c| c.kind() == NodeKind::TypeRef)
            .filter_map(|c| c.text())
            .collect();
        self.types.record_conformance(type_name, conf);
    }

    /// Integer generic parameter names from a nominal's `<...>` clause:
    /// entries spelled `let N: Int` (SE-0452), in declaration order.
    fn value_generic_param_names(&self, node: &Node<'static>) -> Vec<String> {
        node.children()
            .filter(|c| c.kind() == NodeKind::GenericParam)
            .filter_map(|c| c.text())
            .flat_map(|text| {
                text.trim_start_matches('<')
                    .trim_end_matches('>')
                    .split(',')
                    .filter_map(|entry| {
                        let rest = entry.trim().strip_prefix("let ")?;
                        Some(rest.split(':').next()?.trim().to_string())
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Register a protocol declaration (name + inherited protocols), recording
    /// `@objc optional` requirement names so a non-implementing conformer's
    /// optional-chained use resolves to `nil` instead of erroring.
    fn register_protocol(&mut self, node: &Node<'static>) {
        let Some(name) = node.text() else { return };
        let inherited: Vec<String> = node
            .children()
            .filter(|c| c.kind() == NodeKind::TypeRef)
            .filter_map(|c| c.text())
            .collect();
        let optional_of = |kinds: &'static [NodeKind]| -> Vec<String> {
            node.children()
                .filter(|c| kinds.contains(&c.kind()) && c.modifier_names().contains(&"optional"))
                .filter_map(|c| c.text().or_else(|| c.decl_name()))
                .collect()
        };
        let methods = optional_of(&[NodeKind::FuncDecl]);
        let properties = optional_of(&[NodeKind::VarDecl, NodeKind::LetDecl]);
        self.types.ensure_protocol(name.clone(), inherited);
        if let Some(def) = self.types.protocol_def_mut(&name) {
            def.optional_methods.extend(methods);
            def.optional_properties.extend(properties);
        }
    }

    /// Register an extension: add its members to the extended type, or — when the
    /// extension targets a protocol — to that protocol's default members. Any
    /// conformances the extension adds are recorded too.
    fn register_extension(&mut self, node: &Node<'static>) {
        let Some(target) = node.text() else { return };
        self.record_conformances(&target, node);
        // Members are the nominal's direct children; there is no synthesized
        // body block. Non-member children (inherited types, attributes, generic
        // params) fall through each loop's `_ => {}` arm.
        let body = node;
        let mut methods = std::collections::HashMap::new();
        let mut computed = std::collections::HashMap::new();
        for member in expand_directives(body) {
            match member.kind() {
                NodeKind::FuncDecl => {
                    if let Some(mname) = member.text() {
                        methods.insert(
                            mname,
                            MethodDef {
                                params: parse_params(&member),
                                body: member.find_child(NodeKind::Block),
                                mutating: member.is_mutating(),
                                generic_params: generic_param_names(&member),
                                is_static: member.is_static(),
                            },
                        );
                    }
                }
                NodeKind::VarDecl | NodeKind::LetDecl => {
                    if let Some(pname) = member.decl_name() {
                        let acc = member.var_accessors();
                        if acc.is_computed {
                            computed.insert(
                                pname,
                                ComputedProp {
                                    getter: acc.getter_body,
                                    setter: acc.setter_body,
                                    setter_param: acc.setter_param,
                                    setter_nonmutating: acc.setter_nonmutating,
                                    is_static: member.is_static(),
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(proto) = self.types.protocol_def_mut(&target) {
            proto.methods.extend(methods);
            proto.computed.extend(computed);
        } else if let Some(def) = self.types.struct_def_mut(&target) {
            def.methods.extend(methods);
            def.computed.extend(computed);
        } else if let Some(def) = self.types.enum_def_mut(&target) {
            def.methods.extend(methods);
            def.computed.extend(computed);
        } else if let Some(def) = self.types.class_def_mut(&target) {
            def.methods.extend(methods);
            def.computed.extend(computed);
        } else {
            // Extension on a builtin type (`extension Int`, `extension Array`,
            // `extension String`, …). Store the members so value-typed
            // receivers can dispatch to them.
            self.types.add_builtin_ext(target, methods, computed);
        }
    }

    /// All protocols a type conforms to, transitively (including protocol
    /// inheritance), for default-implementation lookup.
    pub(super) fn all_protocols(&self, type_name: &str) -> Vec<String> {
        self.types.all_protocols(type_name)
    }

    /// Whether `member` is an `@objc optional` *method* requirement of any
    /// protocol `type_name` conforms to — an unimplemented one called on a
    /// conformer resolves to `nil` (plain or chained access alike; the parser
    /// drops the `?`, so no chain marker survives).
    pub(super) fn protocol_optional_method(&self, type_name: &str, member: &str) -> bool {
        self.all_protocols(type_name).iter().any(|p| {
            self.types
                .protocol_def(p)
                .is_some_and(|d| d.optional_methods.iter().any(|m| m == member))
        })
    }

    /// Whether `member` is an `@objc optional` *property* requirement of any
    /// protocol `type_name` conforms to — an unimplemented one reads as `nil`.
    pub(super) fn protocol_optional_property(&self, type_name: &str, member: &str) -> bool {
        self.all_protocols(type_name).iter().any(|p| {
            self.types
                .protocol_def(p)
                .is_some_and(|d| d.optional_properties.iter().any(|m| m == member))
        })
    }

    /// A protocol default method for `type_name`'s `method`, if any.
    pub(super) fn protocol_default_method(
        &self,
        type_name: &str,
        method: &str,
    ) -> Option<MethodOverload> {
        for proto in self.all_protocols(type_name) {
            if let Some(m) = self
                .types
                .protocol_def(&proto)
                .and_then(|d| d.methods.get(method))
            {
                return Some((
                    clone_params(&m.params),
                    m.body,
                    m.mutating,
                    m.generic_params.clone(),
                ));
            }
        }
        None
    }

    /// Render a value honouring `CustomStringConvertible.description` when the
    /// value's type provides one; otherwise fall back to the plain rendering.
    pub(super) fn render_description(&mut self, value: &SwiftValue) -> String {
        let described = match value {
            SwiftValue::Struct(o) => {
                // User-defined computed `description` takes priority.
                let user_desc = self
                    .types
                    .struct_def(&o.type_name)
                    .is_some_and(|d| d.computed.contains_key("description"))
                    .then(|| self.read_struct_member(value, "description").ok())
                    .flatten();
                // Fall back to a registered builtin property `description`.
                if user_desc.is_none() {
                    let kind = crate::stdlib::BuiltinReceiver::of(value);
                    kind.and_then(|k| {
                        self.builtins.property(k, "description").and_then(|t| {
                            self.module_symbol_visible(t.module)
                                .then_some(t.value)
                                .and_then(|f| f(value.clone()).ok())
                        })
                    })
                } else {
                    user_desc
                }
            }
            SwiftValue::Object(o) => {
                let cn = o.borrow().class_name.clone();
                // User-defined computed `description` takes priority.
                let user_desc = self
                    .class_computed_getter(&cn, "description")
                    .is_some()
                    .then(|| self.read_object_member(value, "description").ok())
                    .flatten();
                // A ClassDef-less builtin Object falls back to a registered
                // builtin `description` (mirrors the `Struct` arm). Absent both,
                // the `Display` impl renders it in struct form
                // (`ClassName(field: value, …)`).
                if user_desc.is_none() && self.types.class_def(&cn).is_none() {
                    let kind = crate::stdlib::BuiltinReceiver::of(value);
                    kind.and_then(|k| {
                        self.builtins.property(k, "description").and_then(|t| {
                            self.module_symbol_visible(t.module)
                                .then_some(t.value)
                                .and_then(|f| f(value.clone()).ok())
                        })
                    })
                } else {
                    user_desc
                }
            }
            SwiftValue::Enum(e) => self
                .types
                .enum_def(&e.type_name)
                .is_some_and(|d| d.computed.contains_key("description"))
                .then(|| self.read_enum_computed(value, "description").ok().flatten())
                .flatten(),
            _ => None,
        };
        match described {
            Some(SwiftValue::Str(s)) => s,
            _ => value.to_string(),
        }
    }

    /// A protocol default computed getter for `type_name`'s `name`, if any.
    pub(super) fn protocol_default_getter(
        &self,
        type_name: &str,
        name: &str,
    ) -> Option<Node<'static>> {
        for proto in self.all_protocols(type_name) {
            if let Some(c) = self
                .types
                .protocol_def(&proto)
                .and_then(|d| d.computed.get(name))
            {
                return c.getter;
            }
        }
        None
    }

    /// Register an enum type from its declaration.
    fn register_enum(&mut self, node: &Node<'static>) {
        let Some(name) = node.text() else { return };
        if self.types.is_enum(&name) {
            return;
        }
        self.record_conformances(&name, node);
        // Members are the nominal's direct children; there is no synthesized
        // body block. Non-member children (inherited types, attributes, generic
        // params) fall through each loop's `_ => {}` arm.
        let body = node;
        // Determine the raw-value backing type from the inherited-type list.
        let raw_kind = node
            .children()
            .filter(|c| c.kind() == NodeKind::TypeRef)
            .find_map(|c| match c.text().as_deref() {
                Some("String") => Some(RawKind::Str),
                Some(t) if IntWidth::from_type_name(t).is_some() => Some(RawKind::Int),
                _ => None,
            });
        let mut next_int: i128 = 0;
        let mut cases = Vec::new();
        let mut methods = std::collections::HashMap::new();
        let mut computed = std::collections::HashMap::new();
        for member in expand_directives(body) {
            match member.kind() {
                // Each `case` element is a flat `EnumCaseDecl(name)`: its
                // expression child (if any) is the raw value (`case c = 1`);
                // its `TypeIdent` children are associated-value types.
                NodeKind::EnumCaseDecl => {
                    let Some(cname) = member.text() else {
                        continue;
                    };
                    let explicit = member
                        .children()
                        .find(|ec| is_expr(ec))
                        .and_then(|n| self.eval(&n).ok());
                    let raw = match raw_kind {
                        Some(RawKind::Int) => {
                            let v = match &explicit {
                                Some(SwiftValue::Int(i)) => i.raw,
                                _ => next_int,
                            };
                            next_int = v + 1;
                            Some(SwiftValue::int(v))
                        }
                        Some(RawKind::Str) => {
                            Some(explicit.unwrap_or_else(|| SwiftValue::Str(cname.clone())))
                        }
                        None => explicit,
                    };
                    let payload_types: Vec<Option<String>> = member
                        .children()
                        .filter(|ec| ec.kind() == NodeKind::TypeRef)
                        .map(|c| c.text())
                        .collect();
                    cases.push(EnumCaseDef {
                        name: cname,
                        raw,
                        payload_types,
                    });
                }
                NodeKind::FuncDecl => {
                    if let Some(mname) = member.text() {
                        methods.insert(
                            mname,
                            MethodDef {
                                params: parse_params(&member),
                                body: member.find_child(NodeKind::Block),
                                mutating: member.is_mutating(),
                                generic_params: generic_param_names(&member),
                                is_static: member.is_static(),
                            },
                        );
                    }
                }
                NodeKind::VarDecl | NodeKind::LetDecl => {
                    if let Some(pname) = member.decl_name() {
                        let acc = member.var_accessors();
                        if acc.is_computed {
                            computed.insert(
                                pname,
                                ComputedProp {
                                    getter: acc.getter_body,
                                    setter: acc.setter_body,
                                    setter_param: acc.setter_param,
                                    setter_nonmutating: acc.setter_nonmutating,
                                    is_static: member.is_static(),
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        self.types.insert_enum(
            name,
            EnumDef {
                cases,
                methods,
                computed,
            },
        );
    }

    /// Register a class type from its declaration.
    fn register_class(&mut self, node: &Node<'static>) {
        let Some(name) = node.text() else { return };
        if self.types.is_class(&name) {
            return;
        }
        self.record_conformances(&name, node);
        // Declaration attributes (`@Model`, …) with the leading `@` already
        // stripped by the frontend, in source order. Surfaced generically via
        // `StdContext::nominal_type_info`.
        let attributes: Vec<String> = node
            .children()
            .filter(|c| c.kind() == NodeKind::Attribute)
            .filter_map(|c| c.text())
            .collect();
        let superclass = node
            .children()
            .find(|c| c.kind() == NodeKind::TypeRef)
            .and_then(|c| c.text());
        // Members are the nominal's direct children; there is no synthesized
        // body block. Non-member children (inherited types, attributes, generic
        // params) fall through each loop's `_ => {}` arm.
        let body = node;
        let mut stored = Vec::new();
        let mut weak_fields = Vec::new();
        let mut computed = std::collections::HashMap::new();
        let mut methods = std::collections::HashMap::new();
        let mut method_overloads: std::collections::HashMap<String, Vec<MethodDef>> =
            std::collections::HashMap::new();
        let mut init = None;
        let mut init_overloads = Vec::new();
        let mut deinit = None;
        let mut static_subscript = None;
        let mut static_inits: Vec<(String, Node<'static>)> = Vec::new();

        for member in expand_directives(body) {
            match member.kind() {
                NodeKind::InitDecl => {
                    let def = MethodDef {
                        params: parse_params(&member),
                        body: member.find_child(NodeKind::Block),
                        mutating: false,
                        generic_params: generic_param_names(&member),
                        is_static: false,
                    };
                    init_overloads.push(clone_method(&def));
                    init = Some(def);
                }
                NodeKind::SubscriptDecl if member.is_static() => {
                    let acc = member.var_accessors();
                    let sbody = acc
                        .getter_body
                        .or_else(|| member.find_child(NodeKind::Block));
                    static_subscript = Some(MethodDef {
                        params: parse_params(&member),
                        body: sbody,
                        mutating: false,
                        generic_params: generic_param_names(&member),
                        is_static: true,
                    });
                }
                NodeKind::DeinitDecl => {
                    deinit = member.find_child(NodeKind::Block);
                }
                NodeKind::FuncDecl => {
                    if let Some(mname) = member.text() {
                        let def = MethodDef {
                            params: parse_params(&member),
                            body: member.find_child(NodeKind::Block),
                            mutating: false,
                            generic_params: generic_param_names(&member),
                            is_static: member.is_static(),
                        };
                        // Accumulate in the overloads vec (retains every
                        // definition even when multiple methods share a name).
                        method_overloads
                            .entry(mname.clone())
                            .or_default()
                            .push(clone_method(&def));
                        // Last-wins map used for single-overload fast path.
                        methods.insert(mname, def);
                    }
                }
                NodeKind::VarDecl | NodeKind::LetDecl => {
                    let Some(pname) = member.decl_name() else {
                        continue;
                    };
                    let acc = member.var_accessors();
                    if acc.is_computed {
                        computed.insert(
                            pname,
                            ComputedProp {
                                getter: acc.getter_body,
                                setter: acc.setter_body,
                                setter_param: acc.setter_param,
                                setter_nonmutating: acc.setter_nonmutating,
                                is_static: member.is_static(),
                            },
                        );
                    } else if member.is_static() {
                        // A `static` stored property is type-level storage; defer
                        // its initializer until the class is registered so it can
                        // reference its own type.
                        if let Some(def) = member.children().find(|c| is_value_node(c)) {
                            static_inits.push((pname.clone(), def));
                        }
                    } else {
                        if member.ownership().as_deref() == Some("weak") {
                            weak_fields.push(pname.clone());
                        }
                        let default = member.children().find(|c| is_value_node(c));
                        let will_set = acc.will_set_body.map(|b| {
                            (
                                acc.will_set_param
                                    .clone()
                                    .unwrap_or_else(|| "newValue".into()),
                                b,
                            )
                        });
                        let did_set = acc.did_set_body.map(|b| {
                            (
                                acc.did_set_param
                                    .clone()
                                    .unwrap_or_else(|| "oldValue".into()),
                                b,
                            )
                        });
                        stored.push(StoredProp {
                            name: pname,
                            ty: field_type_name(&member),
                            default,
                            lazy: member.is_lazy(),
                            will_set,
                            did_set,
                        });
                    }
                }
                _ => {}
            }
        }
        self.types.insert_class(
            name.clone(),
            ClassDef {
                superclass,
                stored,
                weak_fields,
                computed,
                methods,
                method_overloads,
                init,
                init_overloads,
                deinit,
                static_subscript,
                attributes,
            },
        );
        // Evaluate static stored-property initializers now the class exists.
        for (pname, def) in static_inits {
            if let Ok(v) = self.eval(&def) {
                self.statics.insert(format!("{name}.{pname}"), v);
            }
        }
    }

    /// Register a struct type from its declaration.
    fn register_struct(&mut self, node: &Node<'static>) {
        let Some(name) = node.text() else { return };
        if self.types.is_struct(&name) {
            return;
        }
        self.record_conformances(&name, node);
        // Declaration attributes (`@Model`, …) with the leading `@` already
        // stripped by the frontend, in source order. Surfaced generically via
        // `StdContext::nominal_type_info`.
        let attributes: Vec<String> = node
            .children()
            .filter(|c| c.kind() == NodeKind::Attribute)
            .filter_map(|c| c.text())
            .collect();
        // `@main` attribute marks the program entry point.
        if attributes.iter().any(|a| a == "main") {
            self.main_type = Some(name.clone());
        }
        // `@dynamicMemberLookup` routes unresolved member access through the
        // type's `subscript(dynamicMember:)`.
        let dynamic_member_lookup = node.children().any(|c| {
            c.kind() == NodeKind::Attribute && c.text().as_deref() == Some("dynamicMemberLookup")
        });
        // `@dynamicCallable` routes call syntax through the type's
        // `dynamicallyCall(...)` method.
        let dynamic_callable = node.children().any(|c| {
            c.kind() == NodeKind::Attribute && c.text().as_deref() == Some("dynamicCallable")
        });
        // Members are the nominal's direct children; there is no synthesized
        // body block. Non-member children (inherited types, attributes, generic
        // params) fall through each loop's `_ => {}` arm.
        let body = node;
        let mut stored = Vec::new();
        let mut computed = std::collections::HashMap::new();
        let mut methods = std::collections::HashMap::new();
        let mut method_overloads: std::collections::HashMap<String, Vec<MethodDef>> =
            std::collections::HashMap::new();
        let mut wrappers = std::collections::HashMap::new();
        let mut subscripts: Vec<SubscriptDef> = Vec::new();
        let mut static_subscript = None;
        let mut init = None;
        let mut init_overloads = Vec::new();
        let mut static_inits: Vec<(String, Node<'static>)> = Vec::new();

        for member in expand_directives(body) {
            match member.kind() {
                NodeKind::InitDecl => {
                    let def = MethodDef {
                        params: parse_params(&member),
                        body: member.find_child(NodeKind::Block),
                        mutating: true,
                        generic_params: generic_param_names(&member),
                        is_static: false,
                    };
                    init_overloads.push(clone_method(&def));
                    init = Some(def);
                }
                NodeKind::SubscriptDecl => {
                    let acc = member.var_accessors();
                    let getter = acc
                        .getter_body
                        .or_else(|| member.find_child(NodeKind::Block));
                    if member.is_static() {
                        static_subscript = Some(MethodDef {
                            params: parse_params(&member),
                            body: getter,
                            mutating: false,
                            generic_params: generic_param_names(&member),
                            is_static: true,
                        });
                    } else {
                        subscripts.push(SubscriptDef {
                            params: parse_params(&member),
                            getter,
                            setter: acc.setter_body,
                            setter_param: acc
                                .setter_param
                                .unwrap_or_else(|| "newValue".to_string()),
                        });
                    }
                }
                NodeKind::FuncDecl => {
                    if let Some(mname) = member.text() {
                        let params = parse_params(&member);
                        let body = member.find_child(NodeKind::Block);
                        let mutating = member.is_mutating();
                        let is_static = member.is_static();
                        let def = MethodDef {
                            params,
                            body,
                            mutating,
                            generic_params: generic_param_names(&member),
                            is_static,
                        };
                        method_overloads
                            .entry(mname.clone())
                            .or_default()
                            .push(clone_method(&def));
                        methods.insert(mname, def);
                    }
                }
                NodeKind::VarDecl | NodeKind::LetDecl => {
                    let Some(pname) = member.decl_name() else {
                        continue;
                    };
                    let is_static = member.is_static();
                    let acc = member.var_accessors();
                    if acc.is_computed {
                        computed.insert(
                            pname,
                            ComputedProp {
                                getter: acc.getter_body,
                                setter: acc.setter_body,
                                setter_param: acc.setter_param,
                                setter_nonmutating: acc.setter_nonmutating,
                                is_static,
                            },
                        );
                    } else {
                        if let Some(attr) =
                            member.children().find(|c| c.kind() == NodeKind::Attribute)
                        {
                            if let Some(type_name) = attr.text() {
                                wrappers.insert(
                                    pname.clone(),
                                    WrapperDef {
                                        type_name,
                                        args: attr.children().collect(),
                                    },
                                );
                            }
                        }
                        let default = member.children().find(|c| is_value_node(c));
                        let will_set = acc.will_set_body.map(|b| {
                            (
                                acc.will_set_param
                                    .clone()
                                    .unwrap_or_else(|| "newValue".into()),
                                b,
                            )
                        });
                        let did_set = acc.did_set_body.map(|b| {
                            (
                                acc.did_set_param
                                    .clone()
                                    .unwrap_or_else(|| "oldValue".into()),
                                b,
                            )
                        });
                        if is_static {
                            // Defer evaluation until after the type is
                            // registered so a static like `static let red =
                            // Color(...)` can reference its own type.
                            if let Some(def) = default {
                                static_inits.push((pname.clone(), def));
                            }
                        } else {
                            stored.push(StoredProp {
                                name: pname,
                                ty: field_type_name(&member),
                                default,
                                lazy: member.is_lazy(),
                                will_set,
                                did_set,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        self.types.insert_struct(
            name.clone(),
            StructDef {
                attributes,
                stored,
                computed,
                methods,
                method_overloads,
                subscripts,
                static_subscript,
                init,
                init_overloads,
                wrappers,
                value_generic_params: self.value_generic_param_names(node),
                dynamic_member_lookup,
                dynamic_callable,
            },
        );
        // Now that the struct is registered, evaluate its static initializers
        // (which may construct instances of the type itself).
        for (pname, def) in static_inits {
            if let Ok(v) = self.eval(&def) {
                self.statics.insert(format!("{name}.{pname}"), v);
            }
        }
    }
}
