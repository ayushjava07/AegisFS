//! The dispatch loop (scheduler) and its worker pool.
//!
//! * [`Dispatcher`] scans the store's ready queue entries, claims each one
//!   (gaining a lease), transitions the run `Queued -> Running`, and submits
//!   the run to the pool.
//! * [`WorkerPool`] is a set of OS threads draining an `mpsc` channel; each
//!   worker executes the attempt via [`RunExecutor`] and then applies the
//!   returned queue action (ack or release-with-delay) while still holding the
//!   claim token.
//!
//! The loop is deterministic: the batch cadence is only a spin tolerance for
//! wall-clock runs, and every decision inside a batch depends only on store
//! contents plus the injected [`Clock`].

use std::sync::mpsc::{self, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crate::clock::Clock;
use crate::domain::ids::RunId;
use crate::domain::status::RunStatus;
use crate::error::StorageError;
use crate::persistence::{ClaimToken, Store};
use crate::plugins::handler::Registry;
use crate::state::run_fsm;

use super::executor::{RunAction, RunExecutor};

/// A claimed run handed to the pool, with the token owning its lease.
#[derive(Debug, Clone)]
pub struct Job {
    /// Run id to execute.
    pub run_id: RunId,
    /// Claim token presented on ack/release.
    pub token: ClaimToken,
}

/// Reason a job could not be handed to the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitError {
    /// The bounded channel is at capacity; the caller should drop the lease.
    Full,
    /// The pool has been shut down.
    Disconnected,
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubmitError::Full => write!(f, "worker pool channel is full"),
            SubmitError::Disconnected => write!(f, "worker pool is shut down"),
        }
    }
}

impl std::error::Error for SubmitError {}

/// Result counters for one dispatch batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DispatchStats {
    /// Queue entries scanned as ready.
    pub scanned: usize,
    /// Successfully claimed and submitted.
    pub claimed: usize,
    /// Claimed but submission/transition failed; claim dropped.
    pub submit_failed: usize,
    /// Scanned but lease contention (another dispatcher won).
    pub contended: usize,
}

/// Pulls ready runs off the store and dispatches them to a worker pool.
pub struct Dispatcher<'a> {
    store: &'a dyn Store,
    clock: &'a dyn Clock,
    batch_size: usize,
    lease_ms: i64,
    pool: WorkerPool,
}

impl<'a> Dispatcher<'a> {
    /// A dispatcher bound to a pool.
    pub fn new(
        store: &'a dyn Store,
        clock: &'a dyn Clock,
        pool: WorkerPool,
        batch_size: usize,
        lease_ms: i64,
    ) -> Self {
        Self {
            store,
            clock,
            pool,
            batch_size,
            lease_ms,
        }
    }

    /// One scan-and-dispatch round. Never blocks on a full pool: excess jobs
    /// count as `submit_failed` and their claims are dropped so the entries
    /// return to the pool for the next round.
    pub fn step(&self) -> DispatchStats {
        let now = self.clock.now_ms();
        let mut stats = DispatchStats::default();
        let ready = match self.store.scan_ready(now, self.batch_size) {
            Ok(ready) => ready,
            Err(_) => return stats, // backend unavailable; retry next poll
        };
        stats.scanned = ready.len();
        for entry in ready {
            let token = ClaimToken::new();
            match self.store.claim(&entry.run_id, &token, now, self.lease_ms) {
                Ok(()) => {
                    let transitioned = self.mark_running(&entry.run_id).is_ok();
                    let submitted = self
                        .pool
                        .submit(Job {
                            run_id: entry.run_id.clone(),
                            token: token.clone(),
                        })
                        .is_ok();
                    if transitioned && submitted {
                        stats.claimed += 1;
                    } else {
                        // Return the lease so the entry is not stuck owned.
                        let _ = self.store.failclaim(&entry.run_id, &token);
                        stats.submit_failed += 1;
                    }
                }
                Err(StorageError::ClaimLost(_)) | Err(StorageError::NotFound(_)) => {
                    stats.contended += 1;
                }
                Err(_) => stats.submit_failed += 1,
            }
        }
        stats
    }

    /// Consumes the dispatcher, returning the pool for draining/shutdown.
    pub fn into_pool(self) -> WorkerPool {
        self.pool
    }

    fn mark_running(&self, run_id: &RunId) -> Result<(), StorageError> {
        let run = self.store.get_run(run_id)?;
        if run.status == RunStatus::Running {
            return Ok(()); // idempotent re-dispatch under double-claim
        }
        if run.status != RunStatus::Queued {
            return Err(StorageError::Backend(format!(
                "cannot dispatch run {run_id} from {}",
                run.status
            )));
        }
        run_fsm::validate(RunStatus::Queued, RunStatus::Running)
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        let mut updated = run;
        updated.status = RunStatus::Running;
        updated.started_at_ms = Some(self.clock.now_ms());
        self.store.put_run(&updated)
    }
}

/// A pool of worker threads executing attempts.
pub struct WorkerPool {
    tx: mpsc::SyncSender<Job>,
    workers: Vec<JoinHandle<()>>,
}

impl WorkerPool {
    /// Spawns `size` worker threads bound to the shared registry/store/clock.
    pub fn spawn(
        size: usize,
        registry: Arc<Registry>,
        store: Arc<dyn Store>,
        clock: Arc<dyn Clock>,
        seed: u64,
    ) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Job>(size * 16);
        let rx = Arc::new(parking_lot::Mutex::new(rx));
        let mut workers = Vec::with_capacity(size);
        for _ in 0..size {
            let rx = Arc::clone(&rx);
            let registry = Arc::clone(&registry);
            let store = Arc::clone(&store);
            let clock = Arc::clone(&clock);
            workers.push(thread::spawn(move || {
                let executor =
                    RunExecutor::new(store.as_ref(), Arc::clone(&registry), clock.as_ref(), seed);
                loop {
                    let job = {
                        let guard = rx.lock();
                        match guard.recv() {
                            Ok(job) => job,
                            Err(_) => break, // channel closed and drained
                        }
                    };
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        executor.attempt_with_token(job.run_id.as_str(), Some(&job.token), 60_000)
                    }));
                    match result {
                        Ok(Ok(outcome)) => apply_action(store.as_ref(), &job, outcome.action),
                        Ok(Err(_)) => {
                            // Return the lease so the run is not lost forever;
                            // the failure is observable via logs/audit.
                            let _ = store.release(&job.run_id, &job.token, clock.now_ms());
                        }
                        Err(_) => {
                            // Panic caught: release lease to prevent orphaned lock,
                            // allowing the worker thread to survive and continue serving.
                            let _ = store.release(&job.run_id, &job.token, clock.now_ms());
                        }
                    }
                }
            }));
        }
        Self { tx, workers }
    }

    /// Submits a job, never blocking; `Err(..)` when the pool is full or shut
    /// down.
    pub fn submit(&self, job: Job) -> Result<(), SubmitError> {
        match self.tx.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Disconnected(_)) => Err(SubmitError::Disconnected),
            Err(TrySendError::Full(_)) => Err(SubmitError::Full),
        }
    }

    /// Number of worker threads.
    pub fn size(&self) -> usize {
        self.workers.len()
    }

    /// Drain signal: waits for in-flight jobs then joins every worker. Call only
    /// once all submitters (including the [`Dispatcher`], which holds the
    /// pool) have gone, otherwise the channel stays open.
    pub fn shutdown(self) {
        drop(self.tx); // close this sender; receivers drain queued work
        for handle in self.workers {
            let _ = handle.join();
        }
    }
}

/// Applies the executor's queue decision while the driver holds the token.
pub fn apply_action(store: &dyn Store, job: &Job, action: RunAction) {
    let _ = match action {
        RunAction::Ack => store.ack(&job.run_id, &job.token),
        RunAction::Release { release_at_ms } => {
            store.release(&job.run_id, &job.token, release_at_ms)
        }
    };
}
