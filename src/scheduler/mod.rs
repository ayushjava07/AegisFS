//! Scheduler and worker pool.
//!
//! * [`executor`] — the deterministic run-attempt engine.
//! * [`pick`] — ready-task selection and skipped-chain collapse.
//! * [`pool`] — the scan/claim dispatcher and the threaded worker pool.

pub mod cron;
pub mod executor;
pub mod pick;
pub mod pool;
pub mod reap;
pub mod throttle;

#[cfg(test)]
mod tests;
