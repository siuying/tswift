# Tree-walker benchmark baseline

Establishes the throughput baseline of the current AST tree-walking interpreter.
This is the evidence the **bytecode-VM go/no-go decision** (issue #11,
[ADR-0002](../../../docs/adr/0002-bytecode-vm-vs-tree-walker.md)) depends on, and
the yardstick any future VM's "measurable speedup" must beat.

## Run

```sh
cargo bench -p quick-swift-cli --bench tree_walker
```

## Phases

A register VM would replace the **evaluation** engine; it does **not** change
msf's analysis. The benchmark separates the two so the decision is read against
the metric a VM actually affects:

| Group | What it times | Role in the decision |
| --- | --- | --- |
| `eval_tree_walker` | interpreter construction + stdlib registration + evaluation of an **already-analyzed** AST | **The go/no-go metric.** A VM must beat this. Analysis is leaked once per case, outside the timed loop, so allocation is not folded into the measurement. |
| `analysis_only` | msf analysis of the source text | Context only. A VM does not change this; reported so eval cost can be read in proportion to the full pipeline. |

End-to-end (analysis + eval per call) is intentionally **not** a benchmarked
group: it would require re-analyzing (and thus leaking) inside the timed loop,
and `analysis_only` shows analysis is ~µs against ~hundreds of ms of eval, so the
end-to-end number is dominated by — and indistinguishable from — `eval_tree_walker`.

## Corpus

The corpus lives in `programs/*.swift`. Each program has an `.expected` sibling
and is validated for correctness by `tests/bench_corpus.rs`. Both the benchmark
and that test enumerate the corpus through the shared discovery in
`support/corpus.rs`, so a benchmark can never silently drift from a program that
no longer produces the right result, nor omit a fixture the test validates.

| Program | Workload it stresses |
| --- | --- |
| `fib_recursion` | function call/return, argument passing, eval dispatch (naive `fib(24)`) |
| `loop_sum` | hot `while` loop: env lookups, integer ops, branch eval (200k iters) |
| `struct_ops` | value-type construction, copies, `mutating` methods (50k iters) |

## Baseline (informational)

Captured 2026-06-25 on the maintainer's machine (`cargo bench`, release).
Absolute numbers vary by hardware and load — re-run locally before comparing
against a VM.

| Program | `eval_tree_walker` (median) | `analysis_only` (median) |
| --- | --- | --- |
| `fib_recursion` | ~181 ms | ~14 µs |
| `loop_sum` | ~225 ms | ~12 µs |
| `struct_ops` | ~283 ms | ~22 µs |

Analysis is ~four orders of magnitude cheaper than evaluation, confirming the VM
go/no-go turns entirely on `eval_tree_walker`. A future register VM would need to
demonstrate a material, repeatable speedup there (and on a realistic target
workload) to satisfy the ADR-0002 "go" criteria.
