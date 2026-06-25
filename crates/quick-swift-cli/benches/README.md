# Tree-walker benchmark baseline

Establishes the throughput baseline of the current AST tree-walking interpreter.
This is the evidence the **bytecode-VM go/no-go decision** (issue #11,
[ADR-0002](../../../docs/adr/0002-bytecode-vm-vs-tree-walker.md)) depends on, and
the yardstick any future VM's "measurable speedup" must beat.

## Run

```sh
cargo bench -p quick-swift-cli --bench tree_walker
```

Each benchmark times the **full pipeline** (msf analysis + tree-walking
evaluation) of one program, discarding output so terminal I/O is not measured.

## Corpus

The corpus lives in `programs/*.swift`. Each program has an `.expected` sibling
and is validated for correctness by `tests/bench_corpus.rs`, so a benchmark can
never silently drift from a program that no longer produces the right result.

| Program | Workload it stresses |
| --- | --- |
| `fib_recursion` | function call/return, argument passing, eval dispatch (naive `fib(24)`) |
| `loop_sum` | hot `while` loop: env lookups, integer ops, branch eval (200k iters) |
| `struct_ops` | value-type construction, copies, `mutating` methods (50k iters) |

## Baseline (informational)

Captured 2026-06-25 on the maintainer's machine (`cargo bench`, release).
Absolute numbers vary by hardware — re-run locally before comparing against a VM.

| Program | Median time / iteration |
| --- | --- |
| `fib_recursion` | ~114 ms |
| `loop_sum` | ~139 ms |
| `struct_ops` | ~179 ms |

A future register VM would need to demonstrate a material, repeatable speedup on
these (and on a realistic target workload) to satisfy the ADR-0002 "go" criteria.
