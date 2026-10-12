//! Transport surface: the HTTP API (and, later, the gRPC mirror).
//!
//! This module owns the boundary between the wire and the core: versioned
//! payloads ([`payloads`]), uniform error envelopes ([`error`]), and the
//! axum router + handlers ([`server`]). Handlers stay thin — parsing,
//! authorization, two or three store calls — so behavioral tests can drive
//! the router directly through `tower::ServiceExt`.

#![warn(missing_docs)]

pub mod error;
pub mod payloads;
pub mod server;

pub use error::{ApiError, ErrorBody, ErrorDetail};
pub use payloads::{Envelope, HealthView, RunQuery, SubmitRunRequest, TaskSpecPayload, WorkflowSpec};
pub use server::{AppState, build_router};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_spec_version_stable() {
        let e = Envelope::of(42);
        assert_eq!(e.spec_version, 1);
    }
}