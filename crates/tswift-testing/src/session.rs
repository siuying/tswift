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
    /// Recorded inside a `withKnownIssue { … }` block: reported distinctly and
    /// does *not* fail the test (plan §1.2 "withKnownIssue").
    pub known: bool,
}

struct Session {
    issues: Vec<RawIssue>,
    /// Set by a `#require` failure — the test body must abort.
    aborted: bool,
    /// Nesting depth of active `withKnownIssue` blocks; while `> 0`, recorded
    /// issues are marked `known`.
    known_depth: usize,
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
            known_depth: 0,
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

/// Whether a recording session is currently open. `#expect`/`#require` used
/// outside an active session is a hard error, not a silent no-op.
pub fn is_active() -> bool {
    SESSION.with(|s| s.borrow().is_some())
}

/// Record a soft failure (`#expect`) or the failing `#require` detail. The
/// issue is marked `known` when recorded inside a `withKnownIssue` block.
pub fn record_issue(message: String, line: u32) {
    SESSION.with(|s| {
        if let Some(session) = s.borrow_mut().as_mut() {
            let known = session.known_depth > 0;
            session.issues.push(RawIssue {
                message,
                line,
                known,
            });
        }
    });
}

/// The number of issues recorded so far in the current session (0 when no
/// session is open). Used by `withKnownIssue` to detect whether its body
/// recorded any failure.
pub fn issue_count() -> usize {
    SESSION.with(|s| {
        s.borrow()
            .as_ref()
            .map_or(0, |session| session.issues.len())
    })
}

/// Enter a `withKnownIssue` block: subsequently recorded issues are marked
/// `known` until the matching [`pop_known`].
pub fn push_known() {
    SESSION.with(|s| {
        if let Some(session) = s.borrow_mut().as_mut() {
            session.known_depth += 1;
        }
    });
}

/// Leave a `withKnownIssue` block.
pub fn pop_known() {
    SESSION.with(|s| {
        if let Some(session) = s.borrow_mut().as_mut() {
            session.known_depth = session.known_depth.saturating_sub(1);
        }
    });
}

/// Whether a `#require` failure has aborted the current session (not yet
/// cleared by [`clear_aborted`]). Used by `withKnownIssue` to tell an
/// already-recorded `#require` abort apart from a plain thrown error, so it
/// does not record a second, mislabeled issue for the same failure.
pub fn is_aborted() -> bool {
    SESSION.with(|s| s.borrow().as_ref().is_some_and(|session| session.aborted))
}

/// Clear the abort flag after a `withKnownIssue` block consumed a `#require`
/// abort (the abort is expected inside a known-issue block, so it must not
/// unwind the whole test body).
pub fn clear_aborted() {
    SESSION.with(|s| {
        if let Some(session) = s.borrow_mut().as_mut() {
            session.aborted = false;
        }
    });
}

/// Mark the current test as aborted by a hard `#require` failure.
///
/// `record_issue` for the failing `#require` always runs first, so even if
/// user code wraps the `#require` in a `do`/`catch` that swallows the
/// `StdError::Throw` sentinel this unwind carries, the issue is already on
/// the session and `run_one` still reports the test as failed — the
/// `aborted` flag only distinguishes this unwind from a genuine uncaught
/// throw, it does not gate pass/fail.
pub fn mark_aborted() {
    SESSION.with(|s| {
        if let Some(session) = s.borrow_mut().as_mut() {
            session.aborted = true;
        }
    });
}
