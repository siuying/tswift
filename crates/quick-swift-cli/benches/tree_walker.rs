//! Tree-walker throughput baseline (issue #11, ADR-0002).
//!
//! Establishes the performance baseline of the current AST tree-walking
//! interpreter on a small corpus of representative workloads. This is the
//! evidence the bytecode-VM go/no-go decision depends on, and the yardstick a
//! future VM's "measurable speedup" must beat.
//!
//! Run with: `cargo bench -p quick-swift-cli`
//!
//! The corpus lives in `benches/programs/*.swift` and is validated for
//! correctness by `tests/bench_corpus.rs`, so a benchmark can never silently
//! drift from a program that no longer produces the expected result.

use std::io::{self, Write};
use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, Criterion};
use msf::Analysis;
use quick_swift_core::Interpreter;

/// The benchmark corpus: `(name, relative-path)` under `benches/programs`.
const PROGRAMS: &[(&str, &str)] = &[
    ("fib_recursion", "benches/programs/fib_recursion.swift"),
    ("loop_sum", "benches/programs/loop_sum.swift"),
    ("struct_ops", "benches/programs/struct_ops.swift"),
];

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

fn read_program(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Analyze + evaluate one program end to end, discarding output.
fn run_program(source: &str) {
    let analysis = Analysis::analyze(source, "bench.swift").expect("analysis succeeds");
    // The interpreter borrows the AST for `'static`. Each iteration leaks one
    // small analysis; benches are short-lived processes so this is bounded.
    let analysis: &'static Analysis = Box::leak(Box::new(analysis));
    let mut sink = Sink;
    let mut interp = Interpreter::new(&mut sink);
    quick_swift_std::install(&mut interp);
    interp.run(analysis).expect("program runs without error");
}

fn bench_tree_walker(c: &mut Criterion) {
    let mut group = c.benchmark_group("tree_walker");
    for (name, rel) in PROGRAMS {
        let source = read_program(rel);
        group.bench_function(*name, |b| b.iter(|| run_program(&source)));
    }
    group.finish();
}

criterion_group!(benches, bench_tree_walker);
criterion_main!(benches);
