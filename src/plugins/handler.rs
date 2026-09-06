//! Handler plugin API and built-in handlers.
//!
//! A task is executed by a handler the workflow definition names. The pool
//! looks the handler up in the [`Registry`] and calls its `execute`. This
//! module defines the contract and ships the built-in set; user-authored
//! plugins (dynamic loading, gRPC forwarding) arrive with the plugins phase.
//!
//! Handlers are plain synchronous functions over the task input: keeping the
//! contract synchronous is what lets the worker pool be a set of OS threads
//! with an `mpsc` rendezvous, which the concurrency tests in Phase 2
//! deliberately exercise with `-race`-equivalent rigor.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value as Json};

use crate::domain::ids::HandlerId;
use crate::domain::status::FailureKind;

/// Handler identification symbol (namespace-qualified, e.g. `runvane.echo`).
pub const RUNVANE_NAMESPACE: &str = "runvane";

/// A cooperative cancellation token passed into tasks and shared with the worker.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl CancellationToken {
    /// Creates a new uncancelled token.
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Signals cancellation to any holder of this token.
    pub fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Returns `true` if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The payload a handler receives: task input plus run context.
#[derive(Debug, Clone)]
pub struct TaskContext<'a> {
    /// Tenant owning the run.
    pub tenant: &'a str,
    /// Workflow definition name.
    pub def_name: &'a str,
    /// Run id.
    pub run_id: &'a str,
    /// Task name.
    pub task_name: &'a str,
    /// Static task input from the definition.
    pub input: &'a Json,
    /// Attempt number (1-based) of the current try.
    pub attempt: u32,
    /// Cooperative cancellation token.
    pub cancel_token: CancellationToken,
}

impl<'a> TaskContext<'a> {
    /// Returns whether the task or its enclosing run has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }
}

/// The outcome of a single handler execution.
#[derive(Debug, Clone, PartialEq)]
pub struct HandlerResult {
    /// Task output when the handler succeeded.
    pub output: Json,
}

/// A handler failure, carrying the classification the retry layer needs.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("handler failure ({kind:?}): {message}")]
pub struct HandlerError {
    /// Machine-readable failure classification.
    pub kind: FailureKind,
    /// Human-readable detail.
    pub message: String,
}

impl HandlerError {
    /// Builds a transient handler error.
    pub fn transient(message: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::TransientFailure,
            message: message.into(),
        }
    }

    /// Builds a permanent handler error.
    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::Rejected,
            message: message.into(),
        }
    }

    /// Builds a cancellation handler error.
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::Cancelled,
            message: message.into(),
        }
    }
}

/// A task executor bound to a handler id.
pub trait Handler: Send + Sync {
    /// Executes the task. Implementations must not block on the network for
    /// orders of magnitude longer than `TaskSpec::timeout_ms`.
    fn execute(&self, ctx: TaskContext<'_>) -> Result<HandlerResult, HandlerError>;

    /// A short description for the plugins/registry endpoint.
    fn description(&self) -> &'static str;
}

/// Thread-safe registry mapping handler ids to their implementations.
#[derive(Clone, Default)]
pub struct Registry {
    handlers: Arc<Mutex<HashMap<HandlerId, Arc<dyn Handler>>>>,
}

impl Registry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a handler under its id.
    pub fn register(&self, id: HandlerId, handler: Arc<dyn Handler>) {
        self.handlers.lock().unwrap().insert(id, handler);
    }

    /// Looks up a handler by id.
    pub fn get(&self, id: &HandlerId) -> Option<Arc<dyn Handler>> {
        self.handlers.lock().unwrap().get(id).cloned()
    }

    /// All registered handler ids (sorted for deterministic output).
    pub fn ids(&self) -> Vec<HandlerId> {
        let mut ids: Vec<HandlerId> = self.handlers.lock().unwrap().keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Installs the built-in runvane handlers.
    pub fn install_builtins(&self) {
        self.register(
            HandlerId::from_validated(format!("{RUNVANE_NAMESPACE}.noop")),
            Arc::new(NoopHandler),
        );
        self.register(
            HandlerId::from_validated(format!("{RUNVANE_NAMESPACE}.echo")),
            Arc::new(EchoHandler),
        );
        self.register(
            HandlerId::from_validated(format!("{RUNVANE_NAMESPACE}.fail")),
            Arc::new(FailHandler),
        );
        self.register(
            HandlerId::from_validated(format!("{RUNVANE_NAMESPACE}.delay")),
            Arc::new(DelayHandler::default()),
        );
        self.register(
            HandlerId::from_validated(format!("{RUNVANE_NAMESPACE}.http")),
            Arc::new(super::exec::HttpTaskHandler::default()),
        );
        self.register(
            HandlerId::from_validated(format!("{RUNVANE_NAMESPACE}.script")),
            Arc::new(super::exec::ScriptTaskHandler),
        );
    }
}

/// Returns the input untouched, wrapped in an envelope.
#[derive(Debug, Default)]
pub struct EchoHandler;

impl Handler for EchoHandler {
    fn execute(&self, ctx: TaskContext<'_>) -> Result<HandlerResult, HandlerError> {
        Ok(HandlerResult {
            output: json!({ "echo": ctx.input.clone(), "task": ctx.task_name }),
        })
    }

    fn description(&self) -> &'static str {
        "echoes the task input back"
    }
}

/// Produces `{"ok": true}` with no side effects.
#[derive(Debug, Default)]
pub struct NoopHandler;

impl Handler for NoopHandler {
    fn execute(&self, _ctx: TaskContext<'_>) -> Result<HandlerResult, HandlerError> {
        Ok(HandlerResult {
            output: json!({ "ok": true }),
        })
    }

    fn description(&self) -> &'static str {
        "no-op success"
    }
}

/// Fails every task unconditionally (used by demolition tests and defect
/// injection).
#[derive(Debug, Default)]
pub struct FailHandler;

impl Handler for FailHandler {
    fn execute(&self, ctx: TaskContext<'_>) -> Result<HandlerResult, HandlerError> {
        Err(HandlerError::permanent(format!(
            "runvane.fail handler failed task {}",
            ctx.task_name
        )))
    }

    fn description(&self) -> &'static str {
        "always fails (demolition helper)"
    }
}

/// Succeeds but sleeps; usable where deterministic wall-clock cost is needed.
#[derive(Debug)]
pub struct DelayHandler {
    /// Sleep duration in milliseconds per execution.
    pub delay_ms: u64,
}

impl Default for DelayHandler {
    fn default() -> Self {
        Self { delay_ms: 50 }
    }
}

impl Handler for DelayHandler {
    fn execute(&self, ctx: TaskContext<'_>) -> Result<HandlerResult, HandlerError> {
        let step = std::time::Duration::from_millis(5);
        let total = std::time::Duration::from_millis(self.delay_ms);
        let start = std::time::Instant::now();
        while start.elapsed() < total {
            if ctx.is_cancelled() {
                return Err(HandlerError::cancelled("delay interrupted by cancellation"));
            }
            std::thread::sleep(step.min(total.saturating_sub(start.elapsed())));
        }
        Ok(HandlerResult {
            output: json!({ "slept_ms": self.delay_ms }),
        })
    }

    fn description(&self) -> &'static str {
        "succeeds after a fixed sleep, supporting cancellation"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        let r = Registry::new();
        r.install_builtins();
        r
    }

    fn ctx<'a>(handler_input: &'a Json) -> TaskContext<'a> {
        TaskContext {
            tenant: "acme",
            def_name: "nightly",
            run_id: "rn_abc",
            task_name: "step",
            input: handler_input,
            attempt: 1,
            cancel_token: CancellationToken::new(),
        }
    }

    #[test]
    fn builtins_are_registered() {
        let r = registry();
        assert_eq!(r.ids().len(), 6);
        assert!(r
            .get(&HandlerId::from_validated("runvane.echo".into()))
            .is_some());
        assert!(r
            .get(&HandlerId::from_validated("runvane.http".into()))
            .is_some());
        assert!(r
            .get(&HandlerId::from_validated("runvane.script".into()))
            .is_some());
    }

    #[test]
    fn unknown_handler_is_missing() {
        let r = registry();
        assert!(r
            .get(&HandlerId::from_validated("runvane.nope".into()))
            .is_none());
    }

    #[test]
    fn echo_returns_input() {
        let input = json!({ "a": 1 });
        let result = registry()
            .get(&HandlerId::from_validated("runvane.echo".into()))
            .unwrap()
            .execute(ctx(&input))
            .unwrap();
        assert_eq!(result.output["echo"], input);
    }

    #[test]
    fn noop_succeeds() {
        let input = json!(null);
        let result = registry()
            .get(&HandlerId::from_validated("runvane.noop".into()))
            .unwrap()
            .execute(ctx(&input))
            .unwrap();
        assert_eq!(result.output["ok"], true);
    }

    #[test]
    fn fail_handler_classifies_permanent() {
        let input = json!(null);
        let err = registry()
            .get(&HandlerId::from_validated("runvane.fail".into()))
            .unwrap()
            .execute(ctx(&input))
            .unwrap_err();
        assert_eq!(err.kind, FailureKind::Rejected);
    }

    #[test]
    fn handler_error_helpers_shape_kind() {
        assert_eq!(
            HandlerError::transient("x").kind,
            FailureKind::TransientFailure
        );
        assert_eq!(HandlerError::permanent("x").kind, FailureKind::Rejected);
        assert_eq!(HandlerError::cancelled("x").kind, FailureKind::Cancelled);
    }

    #[test]
    fn cancellation_token_cancels_delay_handler() {
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let mut context = ctx(&json!(null));
        context.cancel_token = token;

        let thread = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(15));
            token_clone.cancel();
        });

        let handler = DelayHandler { delay_ms: 200 };
        let err = handler.execute(context).unwrap_err();
        assert_eq!(err.kind, FailureKind::Cancelled);
        let _ = thread.join();
    }
}
