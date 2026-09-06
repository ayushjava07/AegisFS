//! The run attempt executor: drives a single run through one dispatch round.
//!
//! A worker pulls a claimed run off the queue and hands it to
//! [`RunExecutor::attempt`], which:
//!
//! 1. loads the run and its definition;
//! 2. marks every currently-ready task `Running` and executes it via the
//!    registered handler;
//! 3. collapses skipped task chains after failures;
//! 4. decides the run's next state through the run machine
//!    (`Succeeded`, `Failed` + terminal, or `Queued` again for a retry);
//! 5. returns an [`AttemptOutcome`] the driver uses to ack/release the queue
//!    entry.
//!
//! All time reads go through the injected [`Clock`]; all randomness through
//! the seeded generator, so a test can replay an entire run deterministically.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::clock::Clock;
use crate::domain::ids::{encode_base32hex, HandlerId, RunId, TaskRunId};
use crate::domain::run::{Run, RunError, TaskRun};
use crate::domain::status::{FailureKind, RunStatus, TaskStatus};
use crate::domain::workflow::WorkflowDef;
use crate::error::StorageError as StoreError;
use crate::persistence::{ClaimToken, Store};
use crate::plugins::handler::{
    CancellationToken, HandlerError, HandlerResult, Registry, TaskContext,
};
use crate::state::{run_fsm, task_fsm};

use super::pick;

/// What the driver must do with the queue entry after an attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAction {
    /// Remove the queue entry (run is terminal).
    Ack,
    /// Release the lease at `release_at_ms` (wall ms), keeping the run queued
    /// for a later attempt; the queue entry is un-leased as a consequence.
    Release {
        /// Earliest wall-clock ms at which the run may be re-claimed.
        release_at_ms: i64,
    },
}

/// The outcome of one attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct AttemptOutcome {
    /// Final run status, as decided by the run machine.
    pub run_status: RunStatus,
    /// Names of tasks that failed in this attempt.
    pub failed_tasks: Vec<String>,
    /// Post-attempt queue action for the driver.
    pub action: RunAction,
    /// Terminal failure detail, when the run ended failed.
    pub error: Option<RunError>,
}

/// Errors a single attempt can encounter for non-application reasons.
#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    /// The run record vanished mid-execution.
    #[error("run {0} lost during execution")]
    RunLost(String),
    /// The definition record vanished mid-execution.
    #[error("definition for run {0} lost during execution")]
    DefinitionLost(String),
    /// A persistence layer failure.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    /// A consistency invariant was violated mid-execution.
    #[error("invariant violation: {0}")]
    Invariant(String),
}

/// Canonical task-run id for `(run, task)`. The id body is the base32hex
/// encoding of `run_id::task_name`, so it is charset-valid for any definition
/// name (task names may contain `w`..`z` and `-`, which plain concatenation
/// would corrupt). This is the single source of truth the submission and
/// executor layers both use.
pub fn task_run_id(run_id: &str, task_name: &str) -> TaskRunId {
    let body = encode_base32hex(format!("{run_id}::{task_name}").as_bytes());
    TaskRunId::from_validated(format!("tr_{body}"))
}

/// Executes attempts against a store, registry, and clock.
pub struct RunExecutor<'a> {
    /// The store to read/write runs and task runs through.
    pub store: &'a dyn Store,
    /// Registered handlers.
    pub registry: Arc<Registry>,
    /// Time source.
    pub clock: &'a dyn Clock,
    /// Seed for retry backoff randomness.
    pub seed: u64,
}

impl<'a> RunExecutor<'a> {
    /// A new executor.
    pub fn new(
        store: &'a dyn Store,
        registry: Arc<Registry>,
        clock: &'a dyn Clock,
        seed: u64,
    ) -> Self {
        Self {
            store,
            registry,
            clock,
            seed,
        }
    }

    fn run_id(&self, id: &str) -> RunId {
        RunId::from_validated(id.to_owned())
    }

    fn load_task(&self, run_id: &RunId, task_name: &str) -> TaskRun {
        let id = task_run_id(run_id.as_str(), task_name);
        self.store.get_task_run(&id).unwrap_or_else(|_| TaskRun {
            id,
            run_id: run_id.clone(),
            task_name: task_name.to_owned(),
            status: TaskStatus::Pending,
            attempts: 0,
            last_error: None,
            started_at_ms: None,
            finished_at_ms: None,
            output: None,
        })
    }

    /// Runs one attempt for `run_id` without holding a specific lease token.
    pub fn attempt(&self, run_id: &str) -> Result<AttemptOutcome, ExecutorError> {
        self.attempt_with_token(run_id, None, 60_000)
    }

    /// Runs one attempt for `run_id`, periodically heartbeating/renewing `token`'s lease.
    pub fn attempt_with_token(
        &self,
        run_id: &str,
        token: Option<&ClaimToken>,
        lease_ms: i64,
    ) -> Result<AttemptOutcome, ExecutorError> {
        let run_id_obj = self.run_id(run_id);
        let run = self
            .store
            .get_run(&run_id_obj)
            .map_err(|_| ExecutorError::RunLost(run_id.to_owned()))?;
        let def = self
            .store
            .get_workflow(&run.tenant, &run.def_name)
            .map_err(|_| ExecutorError::DefinitionLost(run_id.to_owned()))?;

        // The dispatcher already transitioned Queued -> Running.
        // Re-read run status: if cancelled between queue claim and attempt, bail early.
        if run.status != RunStatus::Running {
            if run.status == RunStatus::Cancelled {
                return Ok(AttemptOutcome {
                    run_status: RunStatus::Cancelled,
                    failed_tasks: vec![],
                    action: RunAction::Ack,
                    error: run.error.clone(),
                });
            }
            return Err(ExecutorError::Invariant(format!(
                "run {run_id} is {} on attempt; expected Running",
                run.status
            )));
        }

        // Enforce the whole-run deadline before touching any task.
        if run.deadline_at_ms.is_some_and(|d| self.clock.now_ms() > d) {
            run_fsm::validate(RunStatus::Running, RunStatus::TimedOut)
                .map_err(|e| ExecutorError::Invariant(e.to_string()))?;
            let mut timed_out = run.clone();
            timed_out.status = RunStatus::TimedOut;
            timed_out.finished_at_ms = Some(self.clock.now_ms());
            timed_out.error = Some(RunError {
                message: format!("run {run_id} exceeded its deadline"),
                kind: FailureKind::Timeout,
                task: None,
                depth: 0,
                attempts: run.attempts,
            });
            self.store
                .put_run(&timed_out)
                .map_err(ExecutorError::Store)?;
            return Ok(AttemptOutcome {
                run_status: RunStatus::TimedOut,
                failed_tasks: vec![],
                action: RunAction::Ack,
                error: timed_out.error.clone(),
            });
        }

        let mut states = self.loaded_states(&run.id, &def.def);
        // Guards against re-executing a task that failed within this very
        // attempt (it becomes `Failed` in `states`, which the picker would
        // otherwise treat as ready again on the next loop iteration).
        let mut executed_this_attempt: BTreeSet<String> = BTreeSet::new();
        let cancel_token = CancellationToken::new();

        loop {
            // Check if the run was cancelled by an operator during attempt execution.
            if let Ok(current_run) = self.store.get_run(&run_id_obj) {
                if current_run.status == RunStatus::Cancelled {
                    cancel_token.cancel();
                    return Ok(AttemptOutcome {
                        run_status: RunStatus::Cancelled,
                        failed_tasks: vec![],
                        action: RunAction::Ack,
                        error: current_run.error,
                    });
                }
            }

            let ready = pick::ready_tasks(&def.def, &states)
                .into_iter()
                .filter(|name| !executed_this_attempt.contains(name))
                .filter(|name| {
                    // A failed task may be retried only while its own budget
                    // remains; otherwise it stays failed so finalize gives up.
                    if !matches!(states.get(name), Some(TaskStatus::Failed)) {
                        return true;
                    }
                    let tr = self.load_task(&run.id, name);
                    let task = def.def.tasks.iter().find(|t| t.name.as_str() == name);
                    let policy = task
                        .map(|t| t.effective_retry(&def.def.retry))
                        .unwrap_or(&def.def.retry);
                    tr.attempts < policy.max_attempts
                })
                .collect::<Vec<String>>();
            if ready.is_empty() {
                break;
            }
            for name in ready {
                executed_this_attempt.insert(name.clone());
                let task = def
                    .def
                    .tasks
                    .iter()
                    .find(|t| t.name == name)
                    .expect("ready set derives from def tasks");

                // Heartbeat/renew the lease before beginning task execution.
                if let Some(t) = token {
                    let _ = self.store.renew_lease(
                        &run.id,
                        t,
                        self.clock.now_ms(),
                        lease_ms,
                    );
                }

                // Load current record for attempt bookkeeping.
                let mut tr = self.load_task(&run.id, &name);
                tr.status = TaskStatus::Running;
                tr.attempts += 1;
                tr.started_at_ms = Some(self.clock.now_ms());
                self.store.put_task_run(&tr).map_err(ExecutorError::Store)?;
                states.insert(name.clone(), TaskStatus::Running);

                let ctx = TaskContext {
                    tenant: &run.tenant,
                    def_name: &run.def_name,
                    run_id,
                    task_name: &name,
                    input: &task.input,
                    attempt: tr.attempts,
                    cancel_token: cancel_token.clone(),
                };

                match self.run_handler(&task.handler, &name, ctx) {
                    Ok(result) => {
                        let mut tr = self.load_task(&run.id, &name);
                        if !task_fsm::is_legal(tr.status, TaskStatus::Succeeded) {
                            return Err(ExecutorError::Invariant(format!(
                                "illegal task transition {} -> Succeeded",
                                tr.status
                            )));
                        }
                        tr.status = TaskStatus::Succeeded;
                        tr.finished_at_ms = Some(self.clock.now_ms());
                        tr.output = Some(result.output);
                        self.store.put_task_run(&tr).map_err(ExecutorError::Store)?;
                        states.insert(name.clone(), TaskStatus::Succeeded);
                    }
                    Err(handler_error) => {
                        let mut tr = self.load_task(&run.id, &name);
                        if !task_fsm::is_legal(tr.status, TaskStatus::Failed) {
                            return Err(ExecutorError::Invariant(format!(
                                "illegal task transition {} -> Failed",
                                tr.status
                            )));
                        }
                        tr.status = TaskStatus::Failed;
                        tr.finished_at_ms = Some(self.clock.now_ms());
                        tr.last_error = Some(handler_error.message.clone());
                        self.store.put_task_run(&tr).map_err(ExecutorError::Store)?;
                        states.insert(name.clone(), TaskStatus::Failed);
                    }
                }
            }
        }

        // Collapse skipped chains, persisting any freshly-skipped tasks.
        let collapsed = pick::pending_to_skip(&def.def, &states);
        for (name, status) in &collapsed {
            if *status == TaskStatus::Skipped && states.get(name) != Some(&TaskStatus::Skipped) {
                let mut tr = self.load_task(&run.id, name);
                tr.status = TaskStatus::Skipped;
                tr.finished_at_ms = Some(self.clock.now_ms());
                self.store.put_task_run(&tr).map_err(ExecutorError::Store)?;
            }
        }

        self.finalize(&run, &def.def, &collapsed)
    }

    fn loaded_states(&self, run_id: &RunId, def: &WorkflowDef) -> BTreeMap<String, TaskStatus> {
        let stored = self
            .store
            .list_task_runs_for_run(run_id)
            .unwrap_or_default();
        let mut states: BTreeMap<String, TaskStatus> = stored
            .iter()
            .map(|t| (t.task_name.clone(), t.status))
            .collect();
        for task in &def.tasks {
            states
                .entry(task.name.clone())
                .or_insert(TaskStatus::Pending);
        }
        states
    }

    fn run_handler(
        &self,
        handler_id: &HandlerId,
        task_name: &str,
        ctx: TaskContext<'_>,
    ) -> Result<HandlerResult, HandlerError> {
        let handler = self.registry.get(handler_id).ok_or_else(|| {
            HandlerError::permanent(format!("unknown handler {handler_id} for {task_name}"))
        })?;
        handler.execute(ctx)
    }

    fn finalize(
        &self,
        run: &Run,
        def: &WorkflowDef,
        states: &BTreeMap<String, TaskStatus>,
    ) -> Result<AttemptOutcome, ExecutorError> {
        let any_failed = states.values().any(|s| *s == TaskStatus::Failed);
        let all_terminal = states.values().all(|s| s.is_terminal());
        if !all_terminal {
            return Err(ExecutorError::Invariant(
                "attempt ended with non-terminal tasks".to_owned(),
            ));
        }

        if !any_failed {
            // All tasks succeeded; no failure was present, so nothing was
            // skipped either (skip requires a failed dependency).
            run_fsm::validate(run.status, RunStatus::Succeeded)
                .map_err(|e| ExecutorError::Invariant(e.to_string()))?;
            let mut final_run = run.clone();
            final_run.status = RunStatus::Succeeded;
            final_run.finished_at_ms = Some(self.clock.now_ms());
            final_run.error = None;
            self.store
                .put_run(&final_run)
                .map_err(ExecutorError::Store)?;
            return Ok(AttemptOutcome {
                run_status: RunStatus::Succeeded,
                failed_tasks: vec![],
                action: RunAction::Ack,
                error: None,
            });
        }

        // Report the first failing task's message on the run.
        let reason = states
            .iter()
            .find(|(_, s)| **s == TaskStatus::Failed)
            .map(|(name, _)| {
                let tr = self.load_task(&run.id, name);
                tr.last_error
                    .clone()
                    .unwrap_or_else(|| "task failed".to_owned())
            })
            .unwrap_or_else(|| "task failed".to_owned());
        let failing: Vec<String> = states
            .iter()
            .filter(|(_, s)| **s == TaskStatus::Failed)
            .map(|(name, _)| name.clone())
            .collect();
        let failing_task = failing.first().cloned();

        // Retry decision: run-level budget, plus every failed task still has
        // its own per-task attempts left. The run counter is incremented only
        // when a retry is scheduled, so the attempt just consumed must be
        // counted explicitly here.
        let run_budget_ok = def.retry.allows_attempt(run.attempts + 1);
        let task_budget_ok = states
            .iter()
            .filter(|(_, s)| **s == TaskStatus::Failed)
            .all(|(name, _)| {
                let tr = self.load_task(&run.id, name);
                let task = def.tasks.iter().find(|t| t.name.as_str() == name);
                let policy = task
                    .map(|t| t.effective_retry(&def.retry))
                    .unwrap_or(&def.retry);
                tr.attempts < policy.max_attempts
            });

        if run_budget_ok && task_budget_ok {
            // The run machine's retry edge is Failed -> Queued; the attempt
            // legitimately failed, so walk Running -> Failed -> Queued.
            run_fsm::validate(run.status, RunStatus::Failed)
                .map_err(|e| ExecutorError::Invariant(e.to_string()))?;
            run_fsm::validate(RunStatus::Failed, RunStatus::Queued)
                .map_err(|e| ExecutorError::Invariant(e.to_string()))?;
            let mut retry_run = run.clone();
            retry_run.attempts += 1;
            retry_run.status = RunStatus::Queued;
            let delay = crate::retry::backoff::next_attempt_at_ms(
                &def.retry,
                retry_run.attempts,
                true,
                self.clock.now_ms(),
                self.seed,
            )
            .unwrap_or(self.clock.now_ms());
            retry_run.next_attempt_at_ms = Some(delay);
            retry_run.error = None;
            self.store
                .put_run(&retry_run)
                .map_err(ExecutorError::Store)?;
            // The driver releases the existing claim; no fresh enqueue here.
            return Ok(AttemptOutcome {
                run_status: RunStatus::Queued,
                failed_tasks: failing,
                action: RunAction::Release {
                    release_at_ms: delay,
                },
                error: None,
            });
        }

        // Terminal failure.
        run_fsm::validate(run.status, RunStatus::Failed)
            .map_err(|e| ExecutorError::Invariant(e.to_string()))?;
        let mut final_run = run.clone();
        final_run.status = RunStatus::Failed;
        final_run.finished_at_ms = Some(self.clock.now_ms());
        final_run.error = Some(RunError {
            message: reason,
            kind: FailureKind::Exhausted,
            task: failing_task,
            depth: 0,
            attempts: run.attempts,
        });
        self.store
            .put_run(&final_run)
            .map_err(ExecutorError::Store)?;
        Ok(AttemptOutcome {
            run_status: RunStatus::Failed,
            failed_tasks: failing,
            action: RunAction::Ack,
            error: final_run.error.clone(),
        })
    }
}
