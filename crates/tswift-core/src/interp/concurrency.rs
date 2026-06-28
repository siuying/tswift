use std::rc::Rc;

use tswift_frontend::{Node, NodeKind};

use super::{trap, CallArg, ClosureDef, Eval, EvalError, Interpreter, Signal};
use crate::value::{StructObj, SwiftValue};

/// A spawned structured-concurrency task: a zero-argument closure producing the
/// task's result, plus the class context it was spawned in and its run state.
pub(super) struct TaskSlot {
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
pub(super) enum ContinuationState {
    /// Handed to the body but not yet resumed.
    Pending,
    /// `resume(...)` stored an outcome that has not been read yet.
    Resumed(Eval),
    /// The enclosing `with*Continuation` already read the value; any further
    /// `resume(...)` is misuse.
    Consumed,
}

impl<'w> Interpreter<'w> {
    /// `await <expr>`: evaluate the operand, then, if it is a task handle, drive
    /// that task to completion and yield its result. Awaiting any other value is
    /// the identity (an `await f()` on an inline `async` call already ran).
    pub(super) fn eval_await(&mut self, node: &Node<'static>) -> Eval {
        let inner = node
            .children()
            .next()
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
        self.current_task
            .last()
            .is_some_and(|&id| self.tasks[id].cancelled)
    }

    pub(super) fn task_cancelled(&self, task_id: usize) -> bool {
        self.tasks[task_id].cancelled
    }

    /// Run every spawned-but-unawaited task to completion. Called at the end of
    /// the program so detached `Task { }` side effects still happen (structured
    /// concurrency guarantees a child finishes before its scope exits; here the
    /// whole program is the outermost scope).
    pub(super) fn drain_pending_tasks(&mut self) -> Result<(), Signal> {
        let mut i = 0;
        while i < self.tasks.len() {
            if matches!(self.tasks[i].state, TaskState::Pending) {
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
                let gid = self.groups.len();
                self.groups.push(Vec::new());
                self.group_cancelled.push(false);
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
                        if method == "addTaskUnlessCancelled" && self.group_cancelled[gid] {
                            return Ok(Some(SwiftValue::Bool(false)));
                        }
                        let tid = self.spawn_task_closure(closure, false);
                        // A child added to an already-cancelled group starts
                        // cancelled (structured-concurrency propagation).
                        if self.group_cancelled[gid] {
                            self.tasks[tid].cancelled = true;
                        }
                        self.groups[gid].push(tid);
                        Ok(Some(SwiftValue::Bool(true)))
                    }
                    "cancelAll" => {
                        self.group_cancelled[gid] = true;
                        for &tid in &self.groups[gid].clone() {
                            self.tasks[tid].cancelled = true;
                        }
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
                        self.tasks[tid].cancelled = true;
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
                if !matches!(self.continuations[cid], ContinuationState::Pending) {
                    return Err(trap("continuation resumed more than once".into()));
                }
                self.continuations[cid] = ContinuationState::Resumed(outcome);
                Ok(Some(SwiftValue::Void))
            }
            _ => Ok(None),
        }
    }

    /// Consume a group's children for `for await`, returning their results in
    /// completion order (our cooperative executor runs them in add order).
    pub(super) fn drain_group_results(&mut self, gid: usize) -> Result<Vec<SwiftValue>, Signal> {
        let ids = std::mem::take(&mut self.groups[gid]);
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
        let id = self.tasks.len();
        let cancelled = inherit
            && self
                .current_task
                .last()
                .is_some_and(|&parent| self.tasks[parent].cancelled);
        self.tasks.push(TaskSlot {
            closure: closure_id,
            class_ctx: self.class_ctx.clone(),
            state: TaskState::Pending,
            cancelled,
        });
        id
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
        match &self.tasks[id].state {
            TaskState::Done(result) => return result.clone(),
            TaskState::Running => {
                return Err(trap("await on a task awaiting itself (deadlock)".into()));
            }
            TaskState::Pending => {}
        }
        let closure = self.tasks[id].closure;
        let ctx = self.tasks[id].class_ctx.clone();
        self.tasks[id].state = TaskState::Running;
        let saved_ctx = std::mem::replace(&mut self.class_ctx, ctx);
        self.current_task.push(id);
        let result = self.call_closure(closure, Vec::new());
        self.current_task.pop();
        self.class_ctx = saved_ctx;
        self.tasks[id].state = TaskState::Done(result.clone());
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
        let cid = self.continuations.len();
        self.continuations.push(ContinuationState::Pending);
        let body_tasks_start = self.tasks.len();
        self.call_closure(body, vec![SwiftValue::Continuation(cid)])?;
        if matches!(self.continuations[cid], ContinuationState::Pending) {
            // The body parked the continuation in a task it spawned; drive only
            // those tasks (in spawn order) until one resumes the continuation.
            self.drive_tasks_until_resumed(cid, body_tasks_start)?;
        }
        // Read the value and mark the slot consumed so a later resume traps.
        match std::mem::replace(&mut self.continuations[cid], ContinuationState::Consumed) {
            ContinuationState::Resumed(result) => result,
            _ => Err(trap(
                "continuation was not resumed before with*Continuation returned".into(),
            )),
        }
    }

    /// Drive the tasks spawned at or after `start` (in spawn order) until the
    /// continuation `cid` is resumed, leaving any unrelated/earlier pending
    /// tasks for normal program-end draining. A spawned task that fails with a
    /// genuine interpreter error propagates; an uncaught Swift `throw` from a
    /// detached task is dropped, matching [`drain_pending_tasks`].
    fn drive_tasks_until_resumed(&mut self, cid: usize, start: usize) -> Result<(), Signal> {
        let mut i = start;
        while i < self.tasks.len() {
            if matches!(self.continuations[cid], ContinuationState::Resumed(_)) {
                break;
            }
            if matches!(self.tasks[i].state, TaskState::Pending) {
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
        let ids = std::mem::take(&mut self.groups[gid]);
        for id in ids {
            if let Err(sig @ Signal::Error(_)) = self.run_task(id) {
                return Err(sig);
            }
        }
        Ok(())
    }
}
