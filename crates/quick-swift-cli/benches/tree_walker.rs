//! Tree-walker execution baseline (issue #11, ADR-0002).
//!
//! Establishes the performance baseline of the current AST tree-walking
//! interpreter on a small corpus of representative workloads. This is the
//! evidence the bytecode-VM go/no-go decision depends on, and the yardstick a
//! future VM's "measurable speedup" must beat.
//!
//! Run with: `cargo bench -p quick-swift-cli --bench tree_walker`
//!
//! ## Phases
//!
//! A future register VM replaces the **evaluation** engine; it does *not*
//! change msf's analysis. So the baseline separates the two:
//!
//! - **`eval_tree_walker`** — interpreter construction, stdlib registration, and
//!   evaluation of an *already-analyzed* AST. This is the metric the VM go/no-go
//!   decision is measured against (a VM must beat this).
//! - **`analysis_only`** — msf analysis of source text, for context. A VM does
//!   not change this number; it is reported so eval cost can be read in
//!   proportion to total pipeline cost.
//!
//! The corpus is discovered by `benches/support/corpus.rs` (shared with
//! `tests/bench_corpus.rs`, which validates each program's output), so a
//! benchmark can never silently drift from a program that no longer produces the
//! expected result, nor omit a fixture the test validates.

use std::io::{self, Write};

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use quick_swift_core::Interpreter;
use quick_swift_frontend::Analysis;

#[path = "support/corpus.rs"]
mod corpus;

/// A `Write` sink that discards output, so benchmarks measure evaluation cost
/// rather than terminal I/O.
struct Sink;
impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Analyze `source` once and leak the result so it satisfies the interpreter's
/// `&'static Analysis` borrow.
///
/// The interpreter borrows the AST for `'static` (it stores `Node<'static>`
/// internally). The leak happens **once per benchmark case**, outside
/// `b.iter`, so steady-state iterations allocate nothing and memory growth is
/// not folded into the measurement. The benchmark process is short-lived, so a
/// handful of leaked analyses is bounded.
fn analyze_leaked(source: &str) -> &'static Analysis {
    let analysis = Analysis::analyze(source, "bench.swift").expect("analysis succeeds");
    Box::leak(Box::new(analysis))
}

/// Evaluate an already-analyzed program, discarding output. Mirrors the runtime
/// half of the CLI's `run` path (construct interpreter, install stdlib, run).
fn eval(analysis: &'static Analysis) {
    let mut sink = Sink;
    let mut interp = Interpreter::new(&mut sink);
    quick_swift_std::install(&mut interp);
    interp.run(analysis).expect("program runs without error");
}

/// Isolated tree-walker execution: the number a VM must beat.
fn bench_eval(c: &mut Criterion) {
    let mut group = c.benchmark_group("eval_tree_walker");
    for program in corpus::discover() {
        let source = program.read_source();
        // Leak once per case, never inside the timed loop.
        let analysis = analyze_leaked(&source);
        group.bench_function(&program.name, |b| b.iter(|| eval(black_box(analysis))));
    }
    group.finish();
}

/// msf analysis cost, reported for context (a VM does not change this).
fn bench_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("analysis_only");
    for program in corpus::discover() {
        let source = program.read_source();
        group.bench_function(&program.name, |b| {
            b.iter(|| {
                let analysis = Analysis::analyze(black_box(&source), "bench.swift")
                    .expect("analysis succeeds");
                black_box(analysis);
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench_eval, bench_analysis);
criterion_main!(benches);
