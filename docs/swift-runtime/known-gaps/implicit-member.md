# Implicit member expressions (`.foo` / `.foo(args)`) — known gaps

Status: implemented for enum cases, static stored properties, and static
factory methods (struct/class/enum) in a contextual position.

## Resolution strategy

The frontend does not annotate the contextual type onto these member nodes in
all positions (e.g. `describe(.custom("green"))` leaves the `MemberExpr`
untyped). Resolution therefore works in two steps, matching the existing
enum-case and static-property paths:

1. Use the node's inferred type if the frontend supplied one.
2. Otherwise fall back to a **unique** declaring type.

## Gap: ambiguity not disambiguated by call-site context

If two distinct types both declare a member with the same name (e.g. two
`static func custom(_:)`), an implicit member `.custom(...)` is reported as
*unresolved* even when the surrounding call's parameter type would
disambiguate it (`func describe(_ c: Color)`).

Closing this needs **contextual type propagation**: pushing the expected
parameter type down into argument evaluation so implicit members can resolve
against it. This is an interpreter-wide change affecting all implicit-member
paths (enum cases, static properties, static methods), so it is deferred as a
single follow-up rather than a per-feature fix.
