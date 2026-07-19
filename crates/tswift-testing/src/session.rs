//! The per-test recording session.
//!
//! `#expect`/`#require` handlers receive only a narrow `StdContext`, so the
//! collected issues live in a thread-local the runner brackets around each
//! test. Single-threaded interpreter (ADR-0005), same pattern as the core
//! thread-local registries. Keeping the whole `TestContext` here means
//! `tswift-core` carries no Swift Testing knowledge (plan §5, R1).

use std::cell::RefCell;

/// A recorded failure, still carrying the raw combined-source line; the runner
/// remaps it to `(file, local_line)` via the program's `Analysis`.
#[derive(Debug, Clone)]
pub struct RawIssue {
    pub message: String,
    pub line: u32,
}

struct Session {
    issues: Vec<RawIssue>,
    /// Set by a `#require` failure — the test body must abort.
    aborted: bool,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Open a fresh recording session for one test. Any previous session is
/// discarded (the runner always pairs `begin`/`end`).
pub fn begin() {
    SESSION.with(|s| {
        *s.borrow_mut() = Some(Session {
            issues: Vec::new(),
            aborted: false,
        });
    });
}

/// Close the current session, returning its recorded issues and whether a
/// `#require` aborted the body.
pub fn end() -> (Vec<RawIssue>, bool) {
    SESSION.with(|s| match s.borrow_mut().take() {
        Some(session) => (session.issues, session.aborted),
        None => (Vec::new(), false),
    })
}

/// Record a soft failure (`#expect`) or the failing `#require` detail.
pub fn record_issue(message: String, line: u32) {
    SESSION.with(|s| {
        if let Some(session) = s.borrow_mut().as_mut() {
            session.issues.push(RawIssue { message, line });
        }
    });
}

/// Mark the current test as aborted by a hard `#require` failure.
pub fn mark_aborted() {
    SESSION.with(|s| {
        if let Some(session) = s.borrow_mut().as_mut() {
            session.aborted = true;
        }
    });
}
