//! Runvane — a durable distributed workflow-orchestration platform.
//!
//! Runvane lets platform teams declare versioned workflow definitions, submit
//! runs over HTTP or gRPC, and observe execution through the scheduler, the
//! webhook/event dispatch layer, and a minimal read-only status dashboard.
//! Task execution is delegated to pluggable handlers shipped as first-party
//! plugins, keeping the core transport- and handler-agnostic.
//!
//! This crate is both the library consumed by the `runvane` binary and the
//! home of the unit/integration test-suite for the whole platform.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

/// Version stamped at build time; kept in one place so the CLI, `/debug`
/// endpoint and the dashboard show the same string.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Human-facing product name used in docs, headers and the dashboard title.
pub const PRODUCT_NAME: &str = "Runvane";

/// Very short description used in `--version` output and logging.
pub const PRODUCT_TAGLINE: &str = "durable distributed workflow orchestration";

/// Core domain model: workflow definitions, runs, statuses, validation.
pub mod domain;

/// Deterministic time abstraction.
pub mod clock;

/// State-machine engine and run/task machines.
pub mod state;

/// Top-level error types.
pub mod error;

/// Durable state and the run queue, with memory and SQLite backends.
pub mod persistence;

/// Retry/backoff computation.
pub mod retry;

/// Handler plugins (registry + built-ins).
pub mod plugins;

/// Scheduler: dispatcher, worker pool, run attempt executor.
pub mod scheduler;

/// HTTP and gRPC API: routers, payloads, and error mapping.
pub mod api;
pub mod auth;

/// Server configuration: defaults, TOML file layer, environment overrides.
pub mod config;

/// `runvane` command-line: serve + control-plane client subcommands.
pub mod cli;

/// Webhook/event subsystem: event model, hook dispatch, differential watcher.
pub mod events;
pub mod telemetry;
