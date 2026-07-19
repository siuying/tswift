//! The structured result of a test run — the public surface the CLI (slice B)
//! renders and turns into an exit code.

use std::time::Duration;

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
    /// Set when analysis failed before any test could run; carries the rendered
    /// diagnostics. A run with a compile error is never a success.
    pub compile_error: Option<String>,
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
    pub fn is_success(&self) -> bool {
        self.compile_error.is_none() && self.failed() == 0
    }
}
