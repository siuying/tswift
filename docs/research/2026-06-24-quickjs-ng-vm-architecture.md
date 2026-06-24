---
date: 2026-06-24
title: "QuickJS-NG VM Architecture — Reference for Future Language Implementation"
source: https://github.com/quickjs-ng/quickjs
scope: Core VM only (value model, object model, bytecode interpreter, GC, compilation)
status: complete
---

# QuickJS-NG VM Architecture

A study of how QuickJS-NG implements its JavaScript virtual machine, written as a
**reference for implementing your own dynamic-language VM**. Each section ends
with **"Lessons / takeaways"** distilling reusable design decisions.

The entire engine lives in a single file: `quickjs.c` (~63K lines). Supporting
files: `quickjs.h` (public API + value model), `quickjs-opcode.h` (254 opcodes),
`quickjs-atom.h` (interned strings), `libregexp.c`, `libunicode.c`, `dtoa.c`.

It is a **stack-based bytecode interpreter** — no JIT — with reference-counting +
cycle-collecting GC and V8-style hidden classes ("shapes").

---

## 1. Value Representation

`JSValue` is a tagged value with three compile-time-selectable layouts
(`quickjs.h` ~line 155):

| Layout | When | Form |
|--------|------|------|
| **Struct** | 64-bit default | `{ JSValueUnion u; int64_t tag; }` (16 bytes). Union: `int32`, `double`, `void *ptr`, `short_big_int`. |
| **NaN-boxing** | 32-bit (`JS_NAN_BOXING`) | single `uint64_t`; doubles stored with `JS_FLOAT64_TAG_ADDEND` offset so all non-float tags live in the quiet-NaN space. |
| **JS_CHECK_JSVALUE** | debug only | pointer-typed alias; non-functional, exists only to catch refcount/ownership bugs at compile time. |

**Tags** (`JS_TAG_*`): negative tags are heap-allocated & reference-counted
(`BIG_INT`, `SYMBOL`, `STRING`, `STRING_ROPE`, `MODULE`, `FUNCTION_BYTECODE`,
`OBJECT`); non-negative tags are immediate, inline values (`INT`, `BOOL`, `NULL`,
`UNDEFINED`, `UNINITIALIZED`, `CATCH_OFFSET`, `EXCEPTION`, `SHORT_BIG_INT`,
`FLOAT64`). Small integers ride unboxed in `JS_TAG_INT`; a double is only
promoted to a heap-free `FLOAT64` when it doesn't fit int32.

**Ownership discipline** is encoded in the type system: `JSValue` parameters
transfer ownership (callee frees), `JSValueConst` borrows (caller frees).
Returning `JSValue` transfers to the caller. The `JS_CHECK_JSVALUE` build makes
these two types incompatible so mistakes become compile errors.

> **Lessons / takeaways**
> - Use a tagged union; keep the two hottest types (small int, the heap pointer)
>   cheapest to test.
> - Keep small integers and "immediate" singletons unboxed — most values in a
>   dynamic program are small ints, booleans, null/undefined.
> - On 32-bit, NaN-boxing into a single 64-bit word saves memory and a branch.
> - Encode ownership in distinct (even if aliased) types so the compiler can
>   help police your refcounting.

---

## 2. Object Model & Hidden Classes (Shapes)

`struct JSObject` (~40/48/72 bytes depending on build, `quickjs.c` ~line 1043):
- GC header overlapped with a bitfield struct (`extensible`, `fast_array`,
  `is_exotic`, `is_constructor`, `is_prototype`...) and a `uint16_t class_id`.
- `JSShape *shape` — the hidden class (proto + property layout).
- `JSProperty *prop` — array of property **values**, parallel to the shape.
- `JSWeakRefRecord *first_weak_ref`.
- A large `union u` specialized per `class_id`: bytecode-function fields,
  C-function fields, typed-array/array-buffer data, map state, regexp, promise,
  generator state, proxy data, etc. One object struct serves *every* builtin.

**Shapes = V8-style hidden classes** (`struct JSShape` ~line 1026):
- Holds `proto`, a hash, and a `JSShapeProperty[]` (atom + flags + `hash_next`).
- Property **names/layout** live in the shape; property **values** live in the
  object's `prop[]`. Objects with the same proto and same property set **share a
  single shape** → tiny per-object footprint and cache-friendly lookups.
- The shape's hash table is stored in memory *before* the struct
  (`prop_hash_end()[-h-1]`) — one allocation, good locality.
- **Shape transitions:** adding a property either reuses a cached transition
  (`find_hashed_shape_prop` against `rt->shape_hash`) or clones the shape
  (`js_clone_shape` + `add_shape_property`). `compact_properties` reclaims
  deleted slots.
- **Lookup:** `find_own_property` (~line 6467) masks the atom into the shape's
  inline hash table and walks a short chain — O(1) typical.

> **Lessons / takeaways**
> - Separate *structure* (shape: names, order, flags, proto) from *storage*
>   (values). Sharing shapes across objects is the single biggest memory win and
>   enables inline caching later.
> - Make property keys interned integers ("atoms") so comparison is a pointer/int
>   compare and you can hash cheaply.
> - One fat tagged union for all builtin object kinds keeps the allocator and GC
>   simple; the `class_id` switches behavior.
> - Co-locate a shape's hash table with the shape allocation for locality.

---

## 3. Atoms (Interned Strings)

Property names, identifiers, and well-known symbols are **atoms** — interned
integers into a runtime-wide table. `quickjs-atom.h` pre-defines the well-known
ones via X-macros (`DEF(name, "str")`) so they're compile-time constants
(`JS_ATOM_xxx`) requiring no lookup. Everything keyed by name (property access,
method dispatch) uses an atom, making comparisons O(1).

> **Lessons / takeaways**
> - Intern all identifiers/keys; compare by id, not by string bytes.
> - Pre-register your language's keywords and builtin names as constant atoms.

---

## 4. Bytecode & Opcodes

254 opcodes declared in `quickjs-opcode.h` with an **X-macro**:
`DEF(id, size, n_pop, n_push, fmt)`. The same header, re-included with different
macro definitions, generates: the opcode enum, the instruction-size table, the
stack-effect tables, and the operand-format enum. Single source of truth.

Notable design choices:
- **Short/quick variants** to shrink hot code: `push_0..push_7`, `push_i8`,
  `get_loc8`, `const8`, etc. encode common operands in 1 byte.
- **"Temporary" opcodes** (`OP_TEMP_START..OP_TEMP_END`) exist only during
  compilation and are rewritten to real opcodes before execution.

> **Lessons / takeaways**
> - Define opcodes in one X-macro list; derive enums, size tables, and the
>   dispatch table from it to avoid drift.
> - Add 1-byte specialized opcodes for the most frequent operations
>   (push small const, load local 0-3, return). Big density/perf win.
> - It's fine to have compiler-only pseudo-ops that never reach the interpreter.

---

## 5. The Interpreter Loop (`JS_CallInternal`, ~line 17580)

**Dispatch** is abstracted behind `SWITCH`/`CASE`/`BREAK`/`DEFAULT` macros with
two implementations:
- `DIRECT_DISPATCH` (GCC/Clang): a computed-goto `dispatch_table[256]` of
  `&&case_OP_x` label addresses. `BREAK` *itself* does the next
  `goto *dispatch_table[*pc++]` — i.e. **threaded code**, no central loop, the
  next-instruction fetch is fused into each handler (better branch prediction).
- Fallback: a plain `switch (opcode = *pc++)` for portability.

**Stack frame** (`JSStackFrame`, ~line 369): per call, one contiguous
`alloca`'d buffer is carved into `arg_buf | var_buf | stack_buf | var_refs`.
`sp` is the operand-stack pointer, `pc` the instruction pointer. Frames link via
`prev_frame` into `rt->current_stack_frame`. `cur_pc` is written back into the
frame *before* any re-entrant call so backtraces and GC stack-walks stay valid.

**Inlined fast paths** (the heart of interpreter performance without a JIT):
- `OP_add` (~line 19608): inlines int32+int32 (overflow → float64) and
  float+float; only otherwise calls `js_add_slow` (string concat, ToPrimitive,
  BigInt). `OP_add_loc` specializes `local += x`.
- `OP_get_field` / `OP_put_field` (~line 19085): inline the object case —
  walk the prototype chain calling `find_own_property`; divert to
  `JS_GetPropertyInternal` only for exotic objects, accessor properties
  (`JS_PROP_TMASK`), or non-objects.
- `OP_call` / `OP_call_method` / `OP_tail_call` (~line 18068): set `cur_pc`,
  recursively call `JS_CallInternal`, free args+callee, push result; tail calls
  `goto done`.

**Entry dispatch:** if the callee isn't an object but has `JS_CALL_FLAG_GENERATOR`,
resume a saved `JSAsyncFunctionState` frame (`goto restart`). Non-bytecode
callables dispatch through `class_array[class_id].call` (C functions, bound
functions, proxies). Bytecode functions allocate the frame and run.

> **Lessons / takeaways**
> - Use computed-goto threaded dispatch where the compiler supports it; keep a
>   `switch` fallback for portability — same handler bodies via macros.
> - One contiguous frame allocation (args+locals+operand stack) is fast to set
>   up and tear down.
> - Save the PC into the frame before any nested call so stack walking, GC, and
>   backtraces work mid-instruction.
> - Inline the int/float arithmetic and the monomorphic property-access path;
>   send everything else to a `_slow` helper. This is 80% of JIT-less speed.

---

## 6. Exception Handling

There is **no separate exception/handler table**. A `try` region pushes a
`JS_TAG_CATCH_OFFSET` value (carrying the catch PC) onto the operand stack.

On error, `goto exception` (~line 20351):
1. Build a backtrace (unless uncatchable).
2. Unwind the operand stack, `JS_FreeValue`-ing each slot.
3. When a `CATCH_OFFSET` slot is popped, jump `pc` to that offset and
   `goto restart` to run the catch/finally. (Offset 0 is a sentinel meaning
   "close this iterator with a throw" — for-of cleanup.)
4. If nothing catches, return `JS_EXCEPTION`.

The shared `done`/`done_generator` epilogue closes any escaped `var_refs` and
frees locals.

> **Lessons / takeaways**
> - Encoding handlers as stack markers (instead of a side table) makes unwinding
>   trivially correct: popping the stack frees temporaries *and* finds the
>   handler in one pass.
> - Reuse the same mechanism for resource cleanup (iterator close, `finally`).

---

## 7. Closures & Variable Capture (`JSVarRef`)

Closures capture variables by `JSVarRef` (the classic "upvalue"):
- **Open:** `pvalue` points directly into the owning frame's stack slot, so the
  closure and the frame share one live binding.
- **Closed:** on frame exit, `close_var_refs` copies the value into the varref
  itself (`value`, `is_detached = 1`) and repoints `pvalue` at it.

Multiple closures over the same variable share one `JSVarRef`, so mutations are
visible to all — matching JS semantics.

> **Lessons / takeaways**
> - Use open/closed upvalues: keep captured locals on the stack until the frame
>   dies, then "close" them onto the heap. Avoids heap-allocating every local.
> - Share one capture cell per variable so aliasing semantics are correct.

---

## 8. Compilation Pipeline (`__JS_EvalInternal`, ~line 37334)

```
source text
  → js_parse_init / skip_shebang
  → js_parse_program            (recursive-descent parser →
                                  JSFunctionDef tree with nested scopes)
  → js_create_function          (emit JSFunctionBytecode for the whole tree)
  → JS_EvalFunctionInternal     (run it)   [or stop early if COMPILE_ONLY]
```

`JSFunctionBytecode` (~line 779) is the compiled artifact:
`byte_code_buf` + `cpool` (constant pool) + `vardefs` (args+locals) +
`closure_var` (capture descriptors) + `stack_size` + `pc2line_buf` (debug line
map) + name/filename/source. Modules additionally run `js_resolve_module`.
`JS_EVAL_FLAG_COMPILE_ONLY` stops after producing the function object — this is
what `qjsc` uses to serialize bytecode ahead of time.

> **Lessons / takeaways**
> - A single-pass recursive-descent parser building a function-def tree, then a
>   bytecode emitter, is plenty for a production dynamic language.
> - Precompute `stack_size` at compile time so the interpreter can allocate the
>   operand stack in one shot (no overflow checks per push).
> - Keep a compact PC→line table for debugging/backtraces instead of embedding
>   positions in the bytecode.
> - Make "compile only" a first-class mode to enable AOT bytecode caching.

---

## 9. Garbage Collection (`JS_RunGC`, ~line 7079)

A **hybrid: reference counting + a cycle collector**.

- Every GC object embeds `JSGCObjectHeader` (`ref_count`, `mark`, a 4-bit
  `gc_obj_type`, and an intrusive list link). Types: JS object, function
  bytecode, shape, var_ref, async function, context.
- **Refcounting** frees the vast majority immediately at count 0
  (`free_zero_refcount`), giving deterministic, prompt destruction.
- **Cycle collection** is a three-phase trial-deletion over `rt->gc_obj_list`:
  1. `gc_decref`: for each object, decrement the refcounts of its children
     (per-type `mark_children`); objects hitting 0 move to `tmp_obj_list`.
  2. `gc_scan`: re-increment from anything still externally reachable, pulling
     genuinely-live objects back out of `tmp_obj_list`.
  3. `gc_free_cycles`: whatever remains is unreachable cyclic garbage — free it,
     staging through `gc_zero_ref_count_list` so finalizers that resurrect/inspect
     "zombie" objects behave safely.
- Triggered when allocation crosses `malloc_gc_threshold` (tunable via
  `JS_SetGCThreshold`).

> **Lessons / takeaways**
> - Refcounting gives predictable latency and prompt frees; add a tracing pass
>   *only* to collect cycles, not for everything.
> - Per-type child-marking callbacks (`mark_children`) keep the collector generic
>   while objects own their traversal logic.
> - Stage cycle frees through a second list to survive finalizer re-entrancy.
> - Drive collection off an allocation threshold, not a timer.

---

## 10. Overall Design Philosophy

QuickJS-NG is a **compact, portable, JIT-less stack VM** that gets surprising
speed from:
1. Unboxed immediates (ints/floats/singletons) in a tagged value.
2. Shapes (hidden classes) for V8-like property access + tiny object footprint.
3. Threaded computed-goto dispatch with inlined fast paths for arithmetic and
   property access.
4. Refcounting for prompt frees + a small cycle collector for correctness.
5. One self-contained C file → trivial embedding, fast startup, small binary.

It deliberately trades peak throughput (no JIT) for **size, portability, startup
time, and embeddability** — the right point on the curve for an embeddable
scripting engine.

### A pragmatic build order for your own VM
1. Tagged value type with unboxed small ints + a couple of singletons.
2. Interned atoms for all keys/identifiers.
3. Object = shape pointer + parallel value array; implement shape transitions.
4. X-macro opcode table → enum + size table + dispatch table.
5. Stack-based interpreter; single contiguous frame; switch dispatch first,
   computed-goto later.
6. Inline int/float arithmetic and monomorphic property access; `_slow` helpers
   for the rest.
7. Stack-marker exception handling (also powers `finally`/iterator close).
8. Open/closed upvalues for closures.
9. Recursive-descent parser → function-def tree → bytecode emitter; precompute
   stack size; emit a PC→line table.
10. Refcounting first; add a trial-deletion cycle collector once cycles appear.

---

## File Map (where to look)

| File | Contents |
|------|----------|
| `quickjs.c` | the entire engine (value, objects, interpreter, GC, builtins, parser) |
| `quickjs.h` | public API + `JSValue` representation and tags |
| `quickjs-opcode.h` | 254 bytecode opcode definitions (X-macros) |
| `quickjs-atom.h` | interned atom (well-known string) definitions |
| `libregexp.c` | regex engine |
| `libunicode.c` | Unicode tables/algorithms |
| `dtoa.c` | number ↔ string conversion |
| `qjsc.c` | AOT bytecode compiler (uses COMPILE_ONLY) |

### Key entry points / line references (as of the studied checkout)
- `JS_CallInternal` — interpreter — ~17580
- `find_own_property` — property lookup — ~6467
- `OP_add` fast path — ~19608
- `OP_get_field` / `OP_put_field` — ~19085
- `OP_call` family — ~18068
- exception unwinder — ~20351
- `__JS_EvalInternal` — compile+run — ~37334
- `JS_RunGC` + `gc_decref`/`gc_scan`/`gc_free_cycles` — ~6959–7099
