//! The suspension primitive (ADR-0004): a suspendable unit of evaluation that
//! runs on its **own native stack** via [`corosensei`], so a deeply-recursive
//! tree-walker computation can pause mid-flight, hand a value back to a
//! scheduler, and later resume *exactly where it left off* — without unwinding
//! the Rust call stack.
//!
//! This is the building block ADR-0005's cooperative executor adopts for
//! preemptive ordering (e.g. `Task.yield()` interleaving, continuations). The
//! `unsafe` stack switching stays confined to `corosensei` (ADR-0001); this
//! module exposes only a safe, typed surface.
//!
//! ```
//! use qswift_core::suspend::{Suspendable, Step, Yielder};
//!
//! // A recursive computation that suspends, yielding progress, then resumes.
//! fn descend(y: &Yielder<i64>, depth: i64, acc: i64) -> i64 {
//!     if depth == 0 {
//!         return acc;
//!     }
//!     // Pause with the running total; the native stack (this recursion) is
//!     // parked in place until `resume` is called again.
//!     y.pause(acc);
//!     descend(y, depth - 1, acc + depth)
//! }
//!
//! let mut task: Suspendable<i64, i64> = Suspendable::new(|y| descend(y, 3, 0));
//! let mut seen = Vec::new();
//! loop {
//!     match task.resume() {
//!         Step::Paused(v) => seen.push(v),
//!         Step::Done(v) => {
//!             seen.push(v);
//!             break;
//!         }
//!     }
//! }
//! assert_eq!(seen, vec![0, 3, 5, 6]);
//! ```

use corosensei::{Coroutine, CoroutineResult, Yielder as RawYielder};

/// The handle a suspendable computation uses to pause itself, yielding an
/// intermediate value of type `Y` back to whoever drives it.
pub struct Yielder<Y = i64> {
    inner: *const RawYielder<(), Y>,
}

impl<Y> Yielder<Y> {
    /// Suspend the running computation, handing `value` back to the driver. The
    /// native stack is parked in place; the next [`Suspendable::resume`] returns
    /// control here and execution continues.
    pub fn pause(&self, value: Y) {
        // SAFETY: `inner` points at the live `corosensei` yielder for this
        // coroutine; it is only ever called on the coroutine's own stack
        // between a `resume`/return pair, so the reference is valid.
        let yielder = unsafe { &*self.inner };
        yielder.suspend(value);
    }
}

/// The outcome of driving a [`Suspendable`] one step.
#[derive(Debug, PartialEq, Eq)]
pub enum Step<Y, R> {
    /// The computation suspended, yielding an intermediate value.
    Paused(Y),
    /// The computation finished, yielding its final result.
    Done(R),
}

/// A computation that can suspend and resume on its own native stack.
pub struct Suspendable<Y = i64, R = i64> {
    coro: Coroutine<(), Y, R>,
    done: bool,
}

impl<Y, R> Suspendable<Y, R> {
    /// Build a suspendable computation from `body`. The body receives a
    /// [`Yielder`] it can use to [`pause`](Yielder::pause) at any depth.
    pub fn new<F>(body: F) -> Self
    where
        F: FnOnce(&Yielder<Y>) -> R + Send + 'static,
        Y: 'static,
        R: 'static,
    {
        let coro = Coroutine::new(move |raw: &RawYielder<(), Y>, ()| {
            let yielder = Yielder { inner: raw };
            body(&yielder)
        });
        Suspendable { coro, done: false }
    }

    /// Drive the computation until its next suspension point or completion.
    ///
    /// # Panics
    /// Panics if called again after it has already returned [`Step::Done`].
    pub fn resume(&mut self) -> Step<Y, R> {
        assert!(!self.done, "resume called on a finished Suspendable");
        match self.coro.resume(()) {
            CoroutineResult::Yield(y) => Step::Paused(y),
            CoroutineResult::Return(r) => {
                self.done = true;
                Step::Done(r)
            }
        }
    }

    /// Whether the computation has run to completion.
    pub fn is_done(&self) -> bool {
        self.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspends_and_resumes_a_recursive_computation() {
        // Proves the core ADR-0004 claim: a recursive (tree-walker-style)
        // computation can pause in the *middle* of its native recursion and
        // resume later with that stack intact.
        let mut task: Suspendable<i64, i64> = Suspendable::new(|y| {
            fn descend(y: &Yielder<i64>, depth: i64, acc: i64) -> i64 {
                if depth == 0 {
                    return acc;
                }
                y.pause(acc);
                descend(y, depth - 1, acc + depth)
            }
            descend(y, 3, 0)
        });

        assert_eq!(task.resume(), Step::Paused(0));
        assert_eq!(task.resume(), Step::Paused(3));
        assert_eq!(task.resume(), Step::Paused(5));
        assert_eq!(task.resume(), Step::Done(6));
        assert!(task.is_done());
    }

    #[test]
    fn round_trips_a_value_across_a_scheduler_boundary() {
        // A trivial "await": the body computes, suspends once (handing a value
        // to the scheduler), and finishes after being resumed.
        let mut task: Suspendable<i64, i64> = Suspendable::new(|y| {
            let partial = 21 * 2;
            y.pause(partial); // hand control back to the "scheduler"
            partial + 1
        });

        assert_eq!(task.resume(), Step::Paused(42));
        assert_eq!(task.resume(), Step::Done(43));
    }
}
