use std::rc::Rc;

use tswift_frontend::{Node, NodeKind};

use super::{trap, CallArg, ClosureDef, Eval, EvalError, Interpreter, Signal};
use crate::value::{StructObj, SwiftValue};

/// A spawned structured-concurrency task: a zero-argument closure producing the
/// task's result, plus the class context it was spawned in and its run state.
struct TaskSlot {
    /// Index into [`Interpreter::closures`] of the task body (a 0-arg thunk).
    closure: usize,
    /// The `super`/`self` dispatch context captured at spawn time.
    class_ctx: Vec<String>,
    state: TaskState,
    /// Cooperative-cancellation flag (set by `cancelAll()` / group teardown).
    cancelled: bool,
}

/// Where a task is in its lifecycle.
enum TaskState {
    /// Spawned but not yet run.
    Pending,
    /// Currently executing (used to detect `await`-on-self deadlock).
    Running,
    /// Finished, carrying its memoized outcome (value or thrown signal).
    Done(Eval),
}

/// Lifecycle of a `with*Continuation` slot. Distinguishing `Resumed` from
/// `Consumed` is what lets the runtime trap on a *late* resume (after the
/// continuation's value has already been read back) the same way it traps on a
/// double resume.
enum ContinuationState {
    /// Handed to the body but not yet resumed.
    Pending,
    /// `resume(...)` stored an outcome that has not been read yet.
    Resumed(Eval),
    /// The enclosing `with*Continuation` already read the value; any further
    /// `resume(...)` is misuse.
    Consumed,
}

/// The outcome of asking the [`Scheduler`] to begin running a task — the result
/// of the `Pending`/`Running`/`Done` transition, decided without evaluating
/// anything. The interpreter turns a [`TaskRun::Start`] into an actual run.
pub(super) enum TaskRun {
    /// The task already finished; here is its memoized outcome.
    Memoized(Eval),
    /// The task is mid-flight on the current stack — awaiting it is a deadlock.
    Deadlock,
    /// The task was pending and is now marked running; evaluate `closure` under
    /// `class_ctx`, then report back via [`Scheduler::complete_task`].
    Start { closure: usize, class_ctx: Vec<String> },
}

/// The structured-concurrency state machine (ADR-0005), owning the task table,
/// the running-task stack, task groups, and continuation slots. Every state
/// transition here is *pure* — it never evaluates Swift — so the subtle
/// invariants (task `Pending`→`Running`→`Done`, continuation
/// `Pending`→`Resumed`→`Consumed`, cancellation propagation) are unit-testable
/// without running a program. The interpreter owns the *driving* loops that
/// call back into evaluation; this owns the bookkeeping they delegate to.
#[derive(Default)]
pub(super) struct Scheduler {
    /// The task table. Each `async let`, `Task { }`, and `group.addTask` pushes
    /// a slot; `await`-ing a `SwiftValue::Task` drives the matching slot.
    tasks: Vec<TaskSlot>,
    /// Stack of currently-executing task ids (innermost last), so
    /// `Task.isCancelled` / `checkCancellation()` reflect the running task's
    /// cooperative-cancellation flag.
    current_task: Vec<usize>,
    /// `withTaskGroup` groups: each holds the task ids added via `addTask`,
    /// drained in order by `for await`.
    groups: Vec<Vec<usize>>,
    /// Per-group cancellation flag (set by `cancelAll()`), so children added
    /// *after* cancellation are spawned cancelled and `addTaskUnlessCancelled`
    /// can refuse to add one.
    group_cancelled: Vec<bool>,
    /// `with*Continuation` slots, one per continuation handed to a body.
    continuations: Vec<ContinuationState>,
}

impl Scheduler {
    // --- task lifecycle ---

    /// Register a 0-argument closure body as a task and return its handle. A
    /// *structured* child (`inherit`) inherits the innermost running task's
    /// cancellation; a detached task starts uncancelled regardless of context.
    fn spawn(&mut self, closure: usize, class_ctx: Vec<String>, inherit: bool) -> usize {
        let cancelled = inherit
            && self
                .current_task
                .last()
                .is_some_and(|&parent| self.tasks[parent].cancelled);
        let id = self.tasks.len();
        self.tasks.push(TaskSlot {
            closure,
            class_ctx,
            state: TaskState::Pending,
            cancelled,
        });
        id
    }

    /// Decide how to run task `id`: return its memoized result if `Done`, flag a
    /// deadlock if it is already `Running`, or transition `Pending`→`Running`
    /// (pushing it onto the running stack) and return the body to evaluate. The
    /// caller must pair a [`TaskRun::Start`] with [`complete_task`].
    fn begin_task(&mut self, id: usize) -> TaskRun {
        match &self.tasks[id].state {
            TaskState::Done(result) => return TaskRun::Memoized(result.clone()),
            TaskState::Running => return TaskRun::Deadlock,
            TaskState::Pending => {}
        }
        let closure = self.tasks[id].closure;
        let class_ctx = self.tasks[id].class_ctx.clone();
        self.tasks[id].state = TaskState::Running;
        self.current_task.push(id);
        TaskRun::Start { closure, class_ctx }
    }

    /// Record task `id`'s outcome and pop it off the running stack, completing
    /// the `Running`→`Done` transition begun by [`begin_task`].
    fn complete_task(&mut self, id: usize, result: Eval) {
        self.current_task.pop();
        self.tasks[id].state = TaskState::Done(result);
    }

    /// The number of spawned tasks (their ids are `0..task_count()`).
    fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// Whether task `id` is still `Pending` (not yet started).
    fn is_task_pending(&self, id: usize) -> bool {
        matches!(self.tasks[id].state, TaskState::Pending)
    }

    /// Whether the innermost running task is cancelled (`false` at top level).
    fn current_task_cancelled(&self) -> bool {
        self.current_task
            .last()
            .is_some_and(|&id| self.tasks[id].cancelled)
    }

    /// Whether task `id` is cancelled.
    fn task_cancelled(&self, id: usize) -> bool {
        self.tasks[id].cancelled
    }

    /// Mark task `id` cancelled (cooperative; checked at `checkCancellation()`).
    fn cancel_task(&mut self, id: usize) {
        self.tasks[id].cancelled = true;
    }

    // --- task groups ---

    /// Open a new task group and return its id.
    fn new_group(&mut self) -> usize {
        let gid = self.groups.len();
        self.groups.push(Vec::new());
        self.group_cancelled.push(false);
        gid
    }

    /// Whether group `gid` has been cancelled via `cancelAll()`.
    fn is_group_cancelled(&self, gid: usize) -> bool {
        self.group_cancelled[gid]
    }

    /// Add child task `tid` to group `gid`; a child added to an already-
    /// cancelled group starts cancelled (structured-concurrency propagation).
    fn add_to_group(&mut self, gid: usize, tid: usize) {
        if self.group_cancelled[gid] {
            self.tasks[tid].cancelled = true;
        }
        self.groups[gid].push(tid);
    }

    /// Cancel group `gid` and every child currently in it.
    fn cancel_group(&mut self, gid: usize) {
        self.group_cancelled[gid] = true;
        for i in 0..self.groups[gid].len() {
            let tid = self.groups[gid][i];
            self.tasks[tid].cancelled = true;
        }
    }

    /// Remove and return group `gid`'s child ids, in add order (for draining).
    fn take_group(&mut self, gid: usize) -> Vec<usize> {
        std::mem::take(&mut self.groups[gid])
    }

    // --- continuations ---

    /// Open a new `Pending` continuation slot and return its id.
    fn new_continuation(&mut self) -> usize {
        let cid = self.continuations.len();
        self.continuations.push(ContinuationState::Pending);
        cid
    }

    /// Whether continuation `cid` is still `Pending` (not yet resumed).
    fn continuation_pending(&self, cid: usize) -> bool {
        matches!(self.continuations[cid], ContinuationState::Pending)
    }

    /// Whether continuation `cid` has been resumed but not yet consumed.
    fn continuation_resumed(&self, cid: usize) -> bool {
        matches!(self.continuations[cid], ContinuationState::Resumed(_))
    }

    /// Store `outcome` on continuation `cid`, transitioning `Pending`→`Resumed`.
    /// Returns `false` (and changes nothing) if the slot is not `Pending` — the
    /// caller turns that into the double/late-resume trap.
    fn resume_continuation(&mut self, cid: usize, outcome: Eval) -> bool {
        if !matches!(self.continuations[cid], ContinuationState::Pending) {
            return false;
        }
        self.continuations[cid] = ContinuationState::Resumed(outcome);
        true
    }

    /// Read continuation `cid`'s outcome and mark it `Consumed` so a later
    /// resume traps. Returns `None` if it was never resumed (the caller traps).
    fn consume_continuation(&mut self, cid: usize) -> Option<Eval> {
        match std::mem::replace(&mut self.continuations[cid], ContinuationState::Consumed) {
            ContinuationState::Resumed(result) => Some(result),
            _ => None,
        }
    }
}

impl<'w> Interpreter<'w> {
    /// `await <expr>`: evaluate the operand, then, if it is a task handle, drive
    /// that task to completion and yield its result. Awaiting any other value is
    /// the identity (an `await f()` on an inline `async` call already ran).
    pub(super) fn eval_await(&mut self, node: &Node<'static>) -> Eval {
        let inner = node
            .first_child()
            .ok_or_else(|| EvalError::Unsupported("await without an operand".into()))?;
        let value = self.eval(&inner)?;
        self.await_value(value)
    }

    /// Spawn a task whose body is a single expression (used by `async let`),
    /// capturing the current lexical scope so the expression sees local state.
    pub(super) fn spawn_expr_task(&mut self, expr: Node<'static>) -> usize {
        let captured = self.env.capture();
        let closure_id = self.closures.len();
        self.closures.push((
            ClosureDef::User {
                params: Vec::new(),
                body: vec![expr],
            },
            captured,
        ));
        self.spawn_task_closure(closure_id, true)
    }

    /// Whether the innermost running task is cancelled (`false` when no task is
    /// on the stack, i.e. top-level code).
    pub(super) fn current_task_cancelled(&self) -> bool {
        self.sched.current_task_cancelled()
    }

    pub(super) fn task_cancelled(&self, task_id: usize) -> bool {
        self.sched.task_cancelled(task_id)
    }

    /// Run every spawned-but-unawaited task to completion. Called at the end of
    /// the program so detached `Task { }` side effects still happen (structured
    /// concurrency guarantees a child finishes before its scope exits; here the
    /// whole program is the outermost scope).
    pub(super) fn drain_pending_tasks(&mut self) -> Result<(), Signal> {
        let mut i = 0;
        while i < self.sched.task_count() {
            if self.sched.is_task_pending(i) {
                if let Err(sig @ Signal::Error(_)) = self.run_task(i) {
                    return Err(sig);
                }
            }
            i += 1;
        }
        Ok(())
    }

    /// Dispatch the free-function concurrency entry points (`Task { }`,
    /// `withTaskGroup { }`). Returns `None` if `name` is not one of them so
    /// normal call resolution continues.
    pub(super) fn try_concurrency_builtin(
        &mut self,
        name: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        match name {
            "Task" => {
                let closure = self
                    .eval_body_closure(arg_nodes)?
                    .ok_or_else(|| EvalError::Unsupported("Task without a body closure".into()))?;
                Ok(Some(SwiftValue::Task(
                    self.spawn_task_closure(closure, true),
                )))
            }
            "withTaskGroup" | "withThrowingTaskGroup" => {
                let body = self.eval_body_closure(arg_nodes)?.ok_or_else(|| {
                    EvalError::Unsupported("withTaskGroup without a body closure".into())
                })?;
                let gid = self.sched.new_group();
                let result = self.call_closure(body, vec![SwiftValue::TaskGroup(gid)]);
                // The group's children are structured: drain any not consumed by
                // a `for await` so they complete before the group returns.
                self.drain_group(gid)?;
                result.map(Some)
            }
            "withCheckedContinuation"
            | "withUnsafeContinuation"
            | "withCheckedThrowingContinuation"
            | "withUnsafeThrowingContinuation" => {
                self.eval_with_continuation(name, arg_nodes).map(Some)
            }
            _ => Ok(None),
        }
    }

    /// Dispatch static methods on the `Task` type. Returns `None` if the base is
    /// not the unshadowed builtin `Task`, so normal method resolution continues.
    pub(super) fn try_task_type_method(
        &mut self,
        base: &Node<'static>,
        method: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        if base.kind() != NodeKind::IdentExpr
            || base.text().as_deref() != Some("Task")
            || self.env.get("Task").is_some()
        {
            return Ok(None);
        }

        let args = self.eval_args(arg_nodes)?;
        match method {
            "detached" => {
                let closure = Self::first_closure(&args).ok_or_else(|| {
                    EvalError::Unsupported("Task.detached without a body closure".into())
                })?;
                // A detached task is not a structured child: it never inherits
                // the spawning task's cancellation.
                Ok(Some(SwiftValue::Task(
                    self.spawn_task_closure(closure, false),
                )))
            }
            // `Task.checkCancellation()` throws `CancellationError` when the
            // running task is cancelled, else does nothing.
            "checkCancellation" => {
                if self.current_task_cancelled() {
                    return Err(Signal::Throw(Self::cancellation_error()));
                }
                Ok(Some(SwiftValue::Void))
            }
            // Cooperative no-ops on our single-threaded executor.
            "yield" | "sleep" => Ok(Some(SwiftValue::Void)),
            _ => Ok(None),
        }
    }

    /// Dispatch instance methods on a task handle or task group. Returns `None`
    /// when `base` is neither, so normal method resolution continues.
    pub(super) fn try_concurrency_method(
        &mut self,
        base: &SwiftValue,
        method: &str,
        arg_nodes: &[Node<'static>],
    ) -> Result<Option<SwiftValue>, Signal> {
        match base {
            SwiftValue::TaskGroup(gid) => {
                let gid = *gid;
                match method {
                    "addTask" | "addTaskUnlessCancelled" => {
                        let args = self.eval_args(arg_nodes)?;
                        let closure = Self::first_closure(&args).ok_or_else(|| {
                            EvalError::Unsupported("addTask without a body closure".into())
                        })?;
                        // `addTaskUnlessCancelled` adds nothing (and returns
                        // `false`) once the group is cancelled.
                        if method == "addTaskUnlessCancelled" && self.sched.is_group_cancelled(gid)
                        {
                            return Ok(Some(SwiftValue::Bool(false)));
                        }
                        let tid = self.spawn_task_closure(closure, false);
                        self.sched.add_to_group(gid, tid);
                        Ok(Some(SwiftValue::Bool(true)))
                    }
                    "cancelAll" => {
                        self.sched.cancel_group(gid);
                        Ok(Some(SwiftValue::Void))
                    }
                    "waitForAll" => {
                        self.drain_group(gid)?;
                        Ok(Some(SwiftValue::Void))
                    }
                    _ => Ok(None),
                }
            }
            SwiftValue::Task(tid) => {
                let tid = *tid;
                match method {
                    "cancel" => {
                        self.sched.cancel_task(tid);
                        Ok(Some(SwiftValue::Void))
                    }
                    _ => Ok(None),
                }
            }
            SwiftValue::Continuation(cid) => {
                let cid = *cid;
                if method != "resume" {
                    return Ok(None);
                }
                let args = self.eval_args(arg_nodes)?;
                let outcome = self.continuation_outcome(&args)?;
                // `CheckedContinuation` traps on a second *or late* resume: only
                // a still-`Pending` slot accepts the outcome.
                if !self.sched.resume_continuation(cid, outcome) {
                    return Err(trap("continuation resumed more than once".into()));
                }
                Ok(Some(SwiftValue::Void))
            }
            _ => Ok(None),
        }
    }

    /// Consume a group's children for `for await`, returning their results in
    /// completion order (our cooperative executor runs them in add order).
    pub(super) fn drain_group_results(&mut self, gid: usize) -> Result<Vec<SwiftValue>, Signal> {
        let ids = self.sched.take_group(gid);
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(self.run_task(id)?);
        }
        Ok(out)
    }

    /// Resolve a value an `await` produced: drive a task handle, pass anything
    /// else through unchanged.
    fn await_value(&mut self, value: SwiftValue) -> Eval {
        match value {
            SwiftValue::Task(id) => self.run_task(id),
            other => Ok(other),
        }
    }

    /// Register a 0-argument closure body as a task and return its handle index.
    /// Spawn a task body. A *structured* child (`inherit = true`) inherits its
    /// enclosing task's cancellation (ADR-0005); a detached task
    /// (`inherit = false`) starts uncancelled regardless of context.
    fn spawn_task_closure(&mut self, closure_id: usize, inherit: bool) -> usize {
        let class_ctx = self.class_ctx.clone();
        self.sched.spawn(closure_id, class_ctx, inherit)
    }

    /// A fresh `CancellationError` value (the stdlib error thrown by
    /// `Task.checkCancellation()`), modelled as an empty conforming struct.
    fn cancellation_error() -> SwiftValue {
        SwiftValue::Struct(Rc::new(StructObj {
            type_name: "CancellationError".into(),
            fields: Vec::new(),
        }))
    }

    /// Drive task `id` to completion (cooperatively, on the current stack) and
    /// return its memoized outcome. Re-awaiting a finished task returns the
    /// stored result; awaiting a task that is mid-flight is a deadlock trap.
    fn run_task(&mut self, id: usize) -> Eval {
        let (closure, ctx) = match self.sched.begin_task(id) {
            TaskRun::Memoized(result) => return result,
            TaskRun::Deadlock => {
                return Err(trap("await on a task awaiting itself (deadlock)".into()));
            }
            TaskRun::Start { closure, class_ctx } => (closure, class_ctx),
        };
        let saved_ctx = std::mem::replace(&mut self.class_ctx, ctx);
        let result = self.call_closure(closure, Vec::new());
        self.class_ctx = saved_ctx;
        self.sched.complete_task(id, result.clone());
        result
    }

    /// The first closure handle among already-evaluated call arguments (the
    /// trailing `{ ... }` of `group.addTask { }`).
    fn first_closure(args: &[CallArg]) -> Option<usize> {
        args.iter().find_map(|a| match a.value {
            SwiftValue::Closure(id) => Some(id),
            _ => None,
        })
    }

    /// Evaluate the trailing body closure of a concurrency call to a closure
    /// handle, ignoring non-closure arguments (e.g. `of: Int.self`).
    fn eval_body_closure(&mut self, arg_nodes: &[Node<'static>]) -> Result<Option<usize>, Signal> {
        for arg in arg_nodes {
            if arg.kind() == NodeKind::ClosureExpr {
                if let SwiftValue::Closure(id) = self.eval(arg)? {
                    return Ok(Some(id));
                }
            }
        }
        Ok(None)
    }

    /// `await with*Continuation { continuation in ... }`: hand the body a
    /// continuation handle, run it, then read back whatever `resume(...)` stored.
    ///
    /// Our executor runs to completion at each `await` (ADR-0005), so the body
    /// either resumes the continuation inline or hands it to a spawned `Task`.
    /// If it is not resumed inline we drive only the tasks *this body spawned*
    /// (not unrelated earlier pending tasks) until the continuation is resumed.
    /// An unresumed continuation traps, mirroring `CheckedContinuation`'s misuse
    /// diagnostic.
    fn eval_with_continuation(&mut self, name: &str, arg_nodes: &[Node<'static>]) -> Eval {
        let body = self
            .eval_body_closure(arg_nodes)?
            .ok_or_else(|| EvalError::Unsupported(format!("{name} without a body closure")))?;
        let cid = self.sched.new_continuation();
        let body_tasks_start = self.sched.task_count();
        self.call_closure(body, vec![SwiftValue::Continuation(cid)])?;
        if self.sched.continuation_pending(cid) {
            // The body parked the continuation in a task it spawned; drive only
            // those tasks (in spawn order) until one resumes the continuation.
            self.drive_tasks_until_resumed(cid, body_tasks_start)?;
        }
        // Read the value and mark the slot consumed so a later resume traps.
        self.sched.consume_continuation(cid).unwrap_or_else(|| {
            Err(trap(
                "continuation was not resumed before with*Continuation returned".into(),
            ))
        })
    }

    /// Drive the tasks spawned at or after `start` (in spawn order) until the
    /// continuation `cid` is resumed, leaving any unrelated/earlier pending
    /// tasks for normal program-end draining. A spawned task that fails with a
    /// genuine interpreter error propagates; an uncaught Swift `throw` from a
    /// detached task is dropped, matching [`drain_pending_tasks`].
    fn drive_tasks_until_resumed(&mut self, cid: usize, start: usize) -> Result<(), Signal> {
        let mut i = start;
        while i < self.sched.task_count() {
            if self.sched.continuation_resumed(cid) {
                break;
            }
            if self.sched.is_task_pending(i) {
                if let Err(sig @ Signal::Error(_)) = self.run_task(i) {
                    return Err(sig);
                }
            }
            i += 1;
        }
        Ok(())
    }

    /// Decode a continuation `resume(...)` call's arguments into the outcome to
    /// store: `resume()` / `resume(returning:)` yield a value; `resume(throwing:)`
    /// a thrown error; `resume(with: .success/.failure)` either, per the `Result`.
    fn continuation_outcome(&self, args: &[CallArg]) -> Result<Eval, Signal> {
        match args.first() {
            // `resume()` - Void continuation.
            None => Ok(Ok(SwiftValue::Void)),
            Some(arg) => match arg.label.as_deref() {
                Some("throwing") => Ok(Err(Signal::Throw(arg.value.clone()))),
                Some("with") => match &arg.value {
                    SwiftValue::Enum(e) if e.case == "success" => {
                        Ok(Ok(e.payload.first().cloned().unwrap_or(SwiftValue::Void)))
                    }
                    SwiftValue::Enum(e) if e.case == "failure" => Ok(Err(Signal::Throw(
                        e.payload.first().cloned().unwrap_or(SwiftValue::Void),
                    ))),
                    other => Err(trap(format!(
                        "resume(with:) expects a Result, got {}",
                        other.type_name()
                    ))),
                },
                // `resume(returning:)` or an unlabeled value.
                _ => Ok(Ok(arg.value.clone())),
            },
        }
    }

    /// Run any still-pending child tasks of group `gid` (structured-concurrency
    /// guarantee: the group does not return until its children finish).
    fn drain_group(&mut self, gid: usize) -> Result<(), Signal> {
        let ids = self.sched.take_group(gid);
        for id in ids {
            if let Err(sig @ Signal::Error(_)) = self.run_task(id) {
                return Err(sig);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A task runs `Pending` → `Running` (via `begin_task`) → `Done` (via
    /// `complete_task`), and a re-`begin` returns the memoized outcome.
    #[test]
    fn task_lifecycle_pending_running_done_memoizes() {
        let mut s = Scheduler::default();
        let id = s.spawn(7, vec!["C".into()], false);
        assert!(s.is_task_pending(id));

        match s.begin_task(id) {
            TaskRun::Start { closure, class_ctx } => {
                assert_eq!(closure, 7);
                assert_eq!(class_ctx, vec!["C".to_string()]);
            }
            _ => panic!("a pending task must Start"),
        }
        // Now Running: a concurrent begin reports a deadlock.
        assert!(!s.is_task_pending(id));
        assert!(matches!(s.begin_task(id), TaskRun::Deadlock));

        s.complete_task(id, Ok(SwiftValue::Bool(true)));
        match s.begin_task(id) {
            TaskRun::Memoized(Ok(SwiftValue::Bool(true))) => {}
            _ => panic!("completed task must memoize its outcome"),
        }
    }

    /// A structured child inherits its parent's cancellation; a detached child
    /// does not.
    #[test]
    fn cancellation_inherits_for_structured_children_only() {
        let mut s = Scheduler::default();
        let parent = s.spawn(0, Vec::new(), false);
        // Enter the parent and cancel it, so it is the innermost running task.
        assert!(matches!(s.begin_task(parent), TaskRun::Start { .. }));
        s.cancel_task(parent);
        assert!(s.current_task_cancelled());

        let structured = s.spawn(1, Vec::new(), true);
        let detached = s.spawn(2, Vec::new(), false);
        assert!(s.task_cancelled(structured), "structured child inherits");
        assert!(!s.task_cancelled(detached), "detached child does not");
    }

    /// A cancelled group cancels its current children and any added afterwards.
    #[test]
    fn group_cancellation_propagates_to_present_and_future_children() {
        let mut s = Scheduler::default();
        let gid = s.new_group();
        let early = s.spawn(0, Vec::new(), false);
        s.add_to_group(gid, early);

        s.cancel_group(gid);
        assert!(s.is_group_cancelled(gid));
        assert!(s.task_cancelled(early), "present child is cancelled");

        let late = s.spawn(1, Vec::new(), false);
        s.add_to_group(gid, late);
        assert!(s.task_cancelled(late), "child added post-cancel starts cancelled");

        assert_eq!(s.take_group(gid), vec![early, late]);
        assert!(s.take_group(gid).is_empty(), "taking a group drains it");
    }

    /// A continuation accepts exactly one resume; the value is read once, and a
    /// second or late resume is rejected.
    #[test]
    fn continuation_resumes_once_then_rejects() {
        let mut s = Scheduler::default();
        let cid = s.new_continuation();
        assert!(s.continuation_pending(cid));

        assert!(s.resume_continuation(cid, Ok(SwiftValue::Bool(true))));
        assert!(s.continuation_resumed(cid));
        // A second resume (still before consume) is rejected.
        assert!(!s.resume_continuation(cid, Ok(SwiftValue::Bool(false))));

        match s.consume_continuation(cid) {
            Some(Ok(SwiftValue::Bool(true))) => {}
            _ => panic!("consume returns the first resumed value"),
        }
        // After consume the slot is gone: a late resume is rejected, and a
        // second consume yields nothing (the caller turns that into a trap).
        assert!(!s.resume_continuation(cid, Ok(SwiftValue::Void)));
        assert!(s.consume_continuation(cid).is_none());
    }

    /// An unresumed continuation has no outcome to consume.
    #[test]
    fn unresumed_continuation_consumes_to_none() {
        let mut s = Scheduler::default();
        let cid = s.new_continuation();
        assert!(s.consume_continuation(cid).is_none());
    }
}
