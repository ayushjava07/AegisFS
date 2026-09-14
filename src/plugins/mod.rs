//! Handler plugins.
//!
//! [handler] defines the executor contract, its errors, the thread-safe
//! registry, and the built-in handlers. Later phases extend this module with
//! dynamic loading and forwarding plugins.

pub mod handler;