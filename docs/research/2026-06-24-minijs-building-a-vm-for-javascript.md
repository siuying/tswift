# MiniJS: How to Build a VM for a Language like JavaScript

**Date:** 2026-06-24
**Subject:** [toprakdeviren/MiniJS](https://github.com/toprakdeviren/MiniJS) — a ~52k-line, zero-dependency JavaScript engine in C11
**Question:** How does MiniJS implement a bytecode VM for a JS-like language, and what is the reusable recipe?

---

## Summary

MiniJS is a complete, classic compiler+VM pipeline rather than a toy tree-walker. It is a good teaching example because every stage is present and readable:

```
source → lexer → parser (AST) → IR generation → register allocation → bytecode → VM loop
              ▲                                                                      │
              └────────── GC, objects/prototypes, env chain, C builtins ◄───────────┘
```

The runtime lives in `src/interp/`. It started life (per the README milestone table) as a **tree-walking evaluator** (`src/interp/eval/`, still present) and later grew a **bytecode VM** (`src/interp/vm.c`) fed by an SSA-style IR with basic blocks and graph-coloring register allocation.

Verified locally:

```
$ make && ./build/minijs --run demo.js   # function add(a,b){return a+b*2}; console.log(add(3,4))
11
```

---

## Stage 1 — Value representation: a tagged union

The foundation of any dynamic-language VM is "what is a value." MiniJS uses a **tagged union** (`JSValue`): a type tag plus a union of payloads (double, string, object pointer, native fn pointer, symbol, BigInt). Everything flows through this single struct.

- NaN-boxing is a deliberate **future** optimization — clarity was chosen first.
- Internal trick (`src/interp/vm.c:5`): an AST `Node*` is smuggled through the `double` slot of a `JSValue` when an opcode needs to carry a pointer.

## Stage 2 — Bytecode design: a register machine

Unlike a stack machine (JVM/CPython), MiniJS is **register-based** (like Lua or JSC's LLInt). The instruction is fixed-width (`src/interp/interp_internal.h:222`):

```c
typedef struct {
    uint8_t  op;          // ~90 opcodes (OP_ADD, OP_CALL, OP_JUMP, ...)
    uint16_t a, b, c;     // register indices / constant indices / jump targets
} Instruction;

struct JSFunctionTemplate {       // one per JS function
    Instruction *code;            // compiled bytecode
    uint32_t     code_len;
    JSValue     *constants;       // constant pool (numbers, strings, var names)
    uint32_t     constant_count;
    uint32_t     reg_count;       // registers this function needs
    const char  *name;
};
```

Operands are register numbers, so `a + b*2` becomes a few `OP_MUL`/`OP_ADD` ops writing into temp registers — no push/pop overhead. The opcode enum (`interp_internal.h:139`) has ~90 entries covering arithmetic, comparison, bitwise, control flow, calls, property access, `try`/`catch`, `for-of`/`for-in`, generators (`OP_YIELD`/`OP_YIELD_FROM`), `OP_AWAIT`, spread, and dynamic import.

## Stage 3 — Why an IR sits between AST and bytecode

The key design decision: AST is **not** lowered straight to bytecode. An **SSA-ish IR with basic blocks** sits in between (`src/interp/ir.h`):

- `IRValue` is one of `IR_VAL_TEMP` (virtual register, unlimited), `IR_VAL_VAR` (a name), or `IR_VAL_CONST`.
- `IRInstr` mirrors the opcodes but uses *virtual* temps and refers to jump targets by **basic-block id**, not byte offset.
- `IRFunction` is a list of `IRBasicBlock`s — i.e. a control-flow graph.

This makes two hard problems easy:

1. **Control flow.** Generating `&&`, `||`, `?:`, loops becomes "make two basic blocks, emit a branch." See `src/interp/irgen/expression.c:460` for `&&`:
   ```c
   IRBasicBlock *rhs_bb  = make_bb(ctx, "and.rhs");
   IRBasicBlock *exit_bb = make_bb(ctx, "and.exit");
   emit_branch(ctx, IR_OP_JUMP_IF_FALSE, lhs_val, exit_bb->id, rhs_bb->id);
   ```
   No manual byte-offset bookkeeping during codegen — blocks are referenced symbolically.

2. **Register allocation.** Virtual temps are unlimited; a later pass squeezes them into a minimal physical register set.

`src/interp/irgen/` recursively walks AST nodes (`gen_expr`) returning the `IRValue` holding each subexpression's result. Entry point: `ir_gen_node()` (`ir.h`), driven from `compile_node_with_scope()` (`src/interp/compile.c:1369`).

## Stage 4 — Register allocation: liveness + graph coloring

The most "real compiler" part, in `src/interp/ir_to_bytecode.c` (`allocate_registers`, starts `:35`). Textbook flow:

1. **Build the CFG** — compute successors per block, including exception-handler edges from `IR_OP_PUSH_TRY` (`:60`).
2. **Liveness analysis** — iterative backward dataflow computing `use`/`def`/`live_in`/`live_out` per block to fixpoint (`:171`).
3. **Interference graph** — two temps interfere if simultaneously live (`interferes[u][v]`, `:208`).
4. **Graph coloring** — greedily assign each temp the lowest color (= physical register) not used by an interfering neighbor (`:~305`). `assert(color < 256)` because call operands are 8-bit.

Result: `reg_count` stays small even for complex functions, because non-overlapping temps **reuse** registers.

## Stage 5 — Lowering IR → linear bytecode (jump patching)

`ir_to_bytecode()` (`src/interp/ir_to_bytecode.c:465`) flattens blocks into a linear instruction array:

- Records `bb_start_pc[bb->id]` = byte offset where each block begins.
- Emits branch instructions with placeholder targets, queuing a `JumpPatch`.
- After all code is emitted, a **patch pass** (`:691`) rewrites every jump's operand from block-id to the real `pc`.

This "emit with holes, backpatch later" technique handles forward jumps.

Calls get special lowering (`:505`): arguments are `OP_MOV`'d into a contiguous register window starting at `max_regs`, and `OP_CALL` packs `dst | (argc<<8)` into operand `a`.

## Stage 6 — The VM loop

`vm_run_internal` (`src/interp/vm.c:78`) is the heart: a `while (pc < code_len) switch (inst.op)` dispatch loop. Each frame:

```c
typedef struct VMFrame {
    JSFunctionTemplate *tmpl;
    JSValue   *regs;     // this frame's register file (malloc'd array)
    uint32_t   pc;
    Env       *env;      // scope chain for variable lookups
    VMFrame   *parent;   // linked list = call stack (also GC roots)
} VMFrame;
```

Simple opcodes:
```c
case OP_LOAD_CONST: regs[inst.a] = tmpl->constants[inst.b]; break;
case OP_MOV:        regs[inst.a] = regs[inst.b];            break;
```

Three implementation choices stand out:

- **Calls recurse the C stack.** `OP_CALL` (`vm.c:532`) gathers args from the register window and calls `call_value` → `call_function` → `vm_run` again (`src/interp/eval/call.c:301`). The JS call stack *is* the C call stack. Simple, but deep JS recursion can overflow the C stack — made a *catchable* error rather than a crash.
- **Exceptions use `setjmp`/`longjmp`.** At function entry the VM `setjmp`s `vm_unwind_jmp` (`vm.c:130`). `throw_value` `longjmp`s to the nearest handler. A `try_stack` of `VMTryFrame`s tracks active `try`/`catch`/`finally` and pending iterator-closes; on a throw the loop walks that stack to find the catch `pc` and resumes. This is how a `TypeError` from a builtin is catchable in user `try/catch`.
- **Generators/async = saved VM state.** `OP_YIELD` (`vm.c:581`) saves `pc`, `env`, the resume register, and the `try_stack` *onto the generator object*, then returns. Resuming re-enters `vm_run_internal` with `generator->generator_regs` (the register file is kept on the heap instead of freed). `async`/`await` reuses the same machinery via a microtask queue. The payoff of a register VM: a coroutine is just "don't free the registers, remember the pc."

## Stage 7 — Supporting runtime

A VM is more than the loop. MiniJS pairs it with:

- **Conservative mark-sweep GC** (`src/interp/gc.c`) — scans the C stack and saved VM registers for roots, so JSValues in C locals mid-expression stay alive without a handle API. Compiled `-O0` to keep locals on the stack. `MINIJS_GC_STRESS=1` collects on every allocation to flush out missed roots.
- **Environments** (`src/interp/env.c`) — runtime scope chain; closures capture by reference. `OP_GET_VAR` walks the chain, falling back to the global object.
- **Objects/prototypes** (`src/interp/object.c`, `access.c`) — property list + prototype link; arrays add a dense element vector.
- **Builtins** (`src/builtin/*`) — the entire stdlib is native C functions (`NativeFn`) registered into the global object; `call_value` dispatches `JS_NATIVE` straight to the C function pointer (`call.c:305`).

---

## Reusable recipe for a JS-like VM

1. **Pick a value representation** — tagged union first, NaN-boxing later.
2. **Lex + parse to an AST** — Pratt parsing with bit-packed precedence.
3. **Generate an IR with basic blocks and unlimited virtual registers** — makes control flow and codegen trivial.
4. **Run liveness + graph-coloring register allocation** — turn virtual temps into a small physical register file.
5. **Lower to fixed-width register bytecode, backpatching jumps.**
6. **Write a `switch`-dispatch VM loop** with per-call frames.
7. **Handle the hard features structurally**: calls recurse C; exceptions via `setjmp`/`longjmp` + a try-frame stack; generators/async by saving and restoring VM frame state.
8. **Add GC + objects + native builtins** around the loop.

**Build order lesson:** MiniJS shipped a tree-walking evaluator first (M0–M6), then added the bytecode VM (post-M6). Get it *running* first, then make it fast.

---

## Key file map

| File | Role |
|---|---|
| `include/minijs.h` | Public API: tokens, AST, arena, scope, JSValue types |
| `src/vm_internal.h` | Engine-private interface |
| `src/lexer.c` | Table-driven tokenizer |
| `src/parser/` | Recursive descent + Pratt parser + SyntaxChecker |
| `src/interp/ir.h` | IR opcodes, `IRValue`/`IRInstr`/`IRBasicBlock`/`IRFunction` |
| `src/interp/irgen/` | AST → IR (expression, statement, pattern, helpers) |
| `src/interp/ir_to_bytecode.c` | Register allocation + IR → bytecode lowering |
| `src/interp/interp_internal.h` | `Opcode` enum, `Instruction`, `JSFunctionTemplate`, `VMFrame` |
| `src/interp/vm.c` | The bytecode interpreter loop (`vm_run_internal`) |
| `src/interp/compile.c` | Drives `ir_gen_node` → `ir_to_bytecode` |
| `src/interp/eval/call.c` | `call_value` dispatch (recurses into VM / native fns) |
| `src/interp/gc.c` | Conservative mark-sweep GC |
| `src/interp/env.c`, `object.c`, `access.c` | Scope chain, objects, prototype walk |
| `src/builtin/*` | Native C standard library |

## Open questions / possible follow-ups

- End-to-end trace of a specific feature (`for-of`, closures, `async`/`await`) from AST node to executed opcodes.
- Annotated bytecode dump for a sample program (no disassembler flag exists today; would need a small tool).
- How module linking (`src/interp/module.c`, `interp_run_module`) interacts with the VM entry path.
