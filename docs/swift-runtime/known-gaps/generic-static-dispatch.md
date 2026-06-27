# Generic static dispatch (`Self` requirements) — known gaps

Status: a free generic function may dispatch through its type parameter to a
static method or stored static property of the bound concrete type
(`func sumAll<T: Addable>(_:) { var acc = T.zero() … }`). Bindings are inferred
from argument values, scoped to the function's declared generic parameters
(parsed from the `GenericParam` AST node), so concrete parameter types such as
`Int8` are never mistaken for placeholders.

## Remaining gaps

1. **Generic methods on types.** `static func make<T>(…) { T.zero() }` declared
   inside a struct/class/enum does not push generic bindings — only free
   functions (`call_function`) do. `call_struct_method` / class dispatch would
   need the same treatment.

2. **Empty-collection inference.** `[T]` binds `T` from the first element, so an
   empty array argument cannot bind `T` unless another parameter does.

3. **Inconsistent same-placeholder bindings.** When several arguments share a
   placeholder, the first binding wins (`or_insert_with`); ill-typed programs
   with heterogeneous values are not diagnosed.

4. **Computed static properties** (`static var defaultValue: Self { … }`) are a
   separate, pre-existing gap: they are not resolved even through a concrete
   type name, so `T.defaultValue` also fails. Stored statics (`static let`)
   work.
