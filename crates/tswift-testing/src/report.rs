//! The structured result of a test run — the public surface the CLI (slice B)
//! renders and turns into an exit code.

use std::time::Duration;

use tswift_frontend::Diagnostic;

/// Why analysis/loading failed before any test could run.
///
/// [`CompileError::Diagnostics`] carries the structured diagnostics straight
/// out of `Analysis` (sema/parse errors) so a caller with the original
/// [`tswift_frontend::SourceFile`]s can render them exactly like `tswift run`
/// does (`file:line:col: error: msg` + source line + caret), rather than a
/// pre-rendered bare string that diverges from that format. Some failure
/// paths (an unparseable manifest, the synthetic test driver failing to
/// build, an interior-NUL source) have no `Analysis` to point at — those stay
/// a plain [`CompileError::Message`].
#[derive(Debug, Clone)]
pub enum CompileError {
    /// Structured error diagnostics from `Analysis`; render with the
    /// program's `SourceFile`s (e.g. `tswift-cli`'s `render_diagnostic`).
    Diagnostics(Vec<Diagnostic>),
    /// A failure with no associated `Analysis`/diagnostics to render.
    Message(String),
}

impl std::fmt::Display for CompileError {
    /// A bare `file:line:col: message` rendering (no `error:`/caret) — good
    /// enough for a log line or an assertion in a test that doesn't have the
    /// original source text handy. A caller that does have the source should
    /// render each [`Diagnostic`] itself for the full swiftc-style output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Diagnostics(diags) => {
                let rendered = diags
                    .iter()
                    .map(|d| {
                        let file = d.file.as_deref().unwrap_or("<input>");
                        format!("{file}:{}:{}: {}", d.line, d.col, d.message)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                f.write_str(&rendered)
            }
            CompileError::Message(msg) => f.write_str(msg),
        }
    }
}

/// A single recorded failure during a test (`#expect`/`#require` failure, an
/// uncaught throw, or a runtime trap). Location is remapped to the originating
/// source file (multi-file concatenation aware).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// Human-readable failure detail (expression spelling + operand values).
    pub message: String,
    /// Originating source file, when known.
    pub file: Option<String>,
    /// 1-based line in `file`.
    pub line: u32,
}

/// The outcome of running one test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStatus {
    Passed,
    Failed,
    /// Reserved for trait-driven skips (slice C).
    Skipped,
}

/// The result of running one discovered `@Test`.
#[derive(Debug, Clone)]
pub struct TestResult {
    /// Fully-qualified id (`"free()"`, `"MathSuite/pass()"`).
    pub id: String,
    /// Display name from `@Test("…")`, if any.
    pub display_name: Option<String>,
    pub status: TestStatus,
    pub issues: Vec<Issue>,
    pub duration: Duration,
    /// Source file the test is declared in.
    pub file: Option<String>,
    /// 1-based declaration line.
    pub line: u32,
}

impl TestResult {
    /// The name to show in reports: the display name if set, else the id.
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.id)
    }
}

/// The whole run: every test's result plus aggregate timing.
#[derive(Debug, Clone, Default)]
pub struct RunReport {
    pub tests: Vec<TestResult>,
    pub duration: Duration,
    /// Set when analysis failed before any test could run. A run with a
    /// compile error is never a success.
    pub compile_error: Option<CompileError>,
}

impl RunReport {
    pub fn passed(&self) -> usize {
        self.count(TestStatus::Passed)
    }

    pub fn failed(&self) -> usize {
        self.count(TestStatus::Failed)
    }

    pub fn skipped(&self) -> usize {
        self.count(TestStatus::Skipped)
    }

    fn count(&self, status: TestStatus) -> usize {
        self.tests.iter().filter(|t| t.status == status).count()
    }

    /// Total recorded issues across every test.
    pub fn issue_count(&self) -> usize {
        self.tests.iter().map(|t| t.issues.len()).sum()
    }

    /// Whether the run should be treated as success (exit 0): analysis
    /// succeeded and no test failed.
    ///
    /// A run that discovered **zero** tests is currently a success (no failures,
    /// no compile error). Whether an empty run should instead be a non-success
    /// (e.g. `swift test`'s "no tests found" exit code) is a CLI policy decision
    /// deferred to slice B, which owns exit-code mapping and can gate on
    /// [`RunReport::tests`] being empty.
    pub fn is_success(&self) -> bool {
        self.compile_error.is_none() && self.failed() == 0
    }
}
