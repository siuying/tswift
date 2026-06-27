# Implicit member expressions (`.foo` / `.foo(args)`) — resolved

Status: implemented for enum cases, static stored properties, and static
factory methods (struct/class/enum) in a contextual position, **including
call-site disambiguation** when several types declare a member of the same
name.

## Resolution strategy

The frontend does not annotate the contextual type onto these member nodes in
all positions (e.g. `describe(.custom("green"))` leaves the `MemberExpr`
untyped). Resolution therefore works in three steps:

1. Use the node's inferred type if the frontend supplied one.
2. Otherwise use the **call-site contextual type** — the declared type of the
   parameter the argument maps to (see below).
3. Otherwise fall back to a **unique** declaring type.

## Call-site contextual type propagation

`eval_args_with` knows the callee's parameters, so before evaluating each
argument it maps that argument to its declared parameter type (positional or
labeled, with variadics keeping their element type) and pushes the type onto a
per-argument hint stack (`Interpreter::type_hint`). The implicit-member
resolvers (`resolve_member_enum`, `resolve_implicit_static`,
`resolve_implicit_static_method`) consult the top of that stack as a contextual
type before falling back to a unique declaring type. The hint is popped after
the argument is evaluated, so nested calls see only their own parameter types.

This closes the former ambiguity gap: if two distinct types both declare a
member with the same name (e.g. two `static func custom(_:)`, two enums with a
`.red` case), an implicit member `.custom(...)` / `.red` now resolves against
the surrounding call's parameter type (`func describe(_ c: Color)`).

## Coverage

- Unit tests: `implicit_static_method_disambiguated_by_param_type`,
  `implicit_enum_case_disambiguated_by_param_type`,
  `implicit_static_property_disambiguated_by_param_type`,
  `implicit_member_disambiguated_in_variadic_arg` in
  `crates/tswift-core/src/interp.rs`.
- Golden fixture:
  `tests/swift-fixtures/tier6-advanced-types/implicit_member_contextual.swift`.
