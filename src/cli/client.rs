//! The `workflows` / `runs` command groups talk to a running control plane
//! over gRPC, reusing the exact protocol stubs the server itself serves.
//! This keeps one wire contract for the whole product surface and lets the
//! CLI be a thin, type-visible shell over it.
//!
//! Each operation returns the raw proto payload; rendering (pretty JSON) is
//! the caller's job so tests can assert on wire fidelity without parsing
//! terminal output.

use tonic::transport::Channel;
use tonic::Code;

use crate::api::grpc::proto::runvane_client::RunvaneClient;
use crate::domain::error::DomainError;
use crate::error::RunvaneError;

/// The generated wire types, re-exported so CLI building code can name them.
pub use crate::api::grpc::proto as wire;

/// Default endpoint when neither `--endpoint` nor `RUNVANE_ENDPOINT` is set.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:9090";

/// A connected gRPC control-plane client.
#[derive(Clone)]
pub struct Client {
    inner: RunvaneClient<Channel>,
}

/// Raised when the wire answer maps to an error we cannot render as JSON.
fn from_tonic(err: tonic::Status) -> RunvaneError {
    match err.code() {
        Code::NotFound => RunvaneError::Domain(DomainError::NotFound {
            entity: "entity",
            id: err.message().to_owned(),
        }),
        Code::InvalidArgument | Code::FailedPrecondition => {
            RunvaneError::Config(err.message().to_owned())
        }
        Code::Unavailable | Code::DeadlineExceeded => RunvaneError::Server(err.message().to_owned()),
        _ => RunvaneError::Server(err.message().to_owned()),
    }
}

impl Client {
    /// Connects to a control-plane endpoint, retrying briefly so a freshly
    /// started `runvane serve` (which binds after config resolution) is not
    /// punished by a connect race.
    pub async fn connect(endpoint: &str) -> Result<Self, RunvaneError> {
        let mut last_error = None;
        for _ in 0..25 {
            match RunvaneClient::connect(endpoint.to_owned()).await {
                Ok(inner) => return Ok(Self { inner }),
                Err(err) => last_error = Some(err),
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Err(RunvaneError::Server(format!(
            "cannot reach control plane at {endpoint}: {last_error:?}"
        )))
    }

    /// `GET /health` in gRPC terms.
    pub async fn health(&mut self) -> Result<wire::HealthResponse, RunvaneError> {
        self.inner
            .health(wire::HealthRequest {})
            .await
            .map(|r| r.into_inner())
            .map_err(from_tonic)
    }

    /// `POST /v1/workflows`.
    pub async fn create_workflow(
        &mut self,
        spec: wire::WorkflowSpec,
    ) -> Result<wire::WorkflowResponse, RunvaneError> {
        self.inner
            .create_workflow(spec)
            .await
            .map(|r| r.into_inner())
            .map_err(from_tonic)
    }

    /// `GET /v1/workflows`.
    pub async fn list_workflows(
        &mut self,
        tenant: &str,
    ) -> Result<wire::ListWorkflowsResponse, RunvaneError> {
        self.inner
            .list_workflows(wire::ListWorkflowsRequest {
                tenant: tenant.to_owned(),
            })
            .await
            .map(|r| r.into_inner())
            .map_err(from_tonic)
    }

    /// `GET /v1/workflows/:tenant/:name`.
    pub async fn get_workflow(
        &mut self,
        tenant: &str,
        name: &str,
    ) -> Result<wire::WorkflowResponse, RunvaneError> {
        self.inner
            .get_workflow(wire::GetWorkflowRequest {
                tenant: tenant.to_owned(),
                name: name.to_owned(),
            })
            .await
            .map(|r| r.into_inner())
            .map_err(from_tonic)
    }

    /// `POST /v1/workflows/:tenant/:name/runs`.
    pub async fn submit_run(
        &mut self,
        tenant: &str,
        name: &str,
        input_json: Vec<u8>,
        tags_json: Vec<u8>,
    ) -> Result<wire::RunResponse, RunvaneError> {
        self.inner
            .submit_run(wire::SubmitRunRequest {
                tenant: tenant.to_owned(),
                name: name.to_owned(),
                input_json,
                tags_json,
            })
            .await
            .map(|r| r.into_inner())
            .map_err(from_tonic)
    }

    /// `GET /v1/runs`.
    pub async fn list_runs(
        &mut self,
        tenant: &str,
        name: &str,
        status: &str,
        limit: u32,
    ) -> Result<wire::ListRunsResponse, RunvaneError> {
        self.inner
            .list_runs(wire::ListRunsRequest {
                tenant: tenant.to_owned(),
                name: name.to_owned(),
                status: status.to_owned(),
                limit,
            })
            .await
            .map(|r| r.into_inner())
            .map_err(from_tonic)
    }

    /// `GET /v1/runs/:id`.
    pub async fn get_run(&mut self, id: &str) -> Result<wire::RunResponse, RunvaneError> {
        self.inner
            .get_run(wire::GetRunRequest { id: id.to_owned() })
            .await
            .map(|r| r.into_inner())
            .map_err(from_tonic)
    }

    /// `POST /v1/runs/:id/cancel`.
    pub async fn cancel_run(&mut self, id: &str) -> Result<wire::RunResponse, RunvaneError> {
        self.inner
            .cancel_run(wire::CancelRunRequest { id: id.to_owned() })
            .await
            .map(|r| r.into_inner())
            .map_err(from_tonic)
    }
}

/// Renders bytes we serialized ourselves as a compact one-line JSON, and any
/// other JSON document under the same rule (stable for diffs and logs).
pub fn render_json(bytes: &[u8]) -> Result<String, RunvaneError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| RunvaneError::Server(e.to_string()))?;
    serde_json::to_string_pretty(&value).map_err(|e| RunvaneError::Server(e.to_string()))
}

/// Renders a list of JSON documents (the gRPC list responses) as a JSON array.
pub fn render_json_list(each: &[Vec<u8>]) -> Result<String, RunvaneError> {
    let mut out = Vec::with_capacity(each.len());
    for bytes in each {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|e| RunvaneError::Server(e.to_string()))?;
        out.push(value);
    }
    serde_json::to_string_pretty(&serde_json::Value::Array(out))
        .map_err(|e| RunvaneError::Server(e.to_string()))
}

/// Parses a JSON document for tagged CLI metadata.
pub fn parse_json_bytes(raw: &str, what: &str) -> Result<Vec<u8>, RunvaneError> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| RunvaneError::Config(format!("{what} does not parse as JSON: {e}")))?;
    serde_json::to_vec(&value).map_err(|e| RunvaneError::Server(e.to_string()))
}

/// Encodes `key=value` tag flags into the `{"k":"v"}` bytes field. Duplicate
/// keys are rejected so an operator cannot silently drop an earlier value.
pub fn tags_to_json(tags: &[String]) -> Result<Vec<u8>, RunvaneError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut map = std::collections::BTreeMap::new();
    for tag in tags {
        let (k, v) = tag
            .split_once('=')
            .ok_or_else(|| RunvaneError::Config(format!("tag {tag:?} must be key=value")))?;
        if !seen.insert(k.to_owned()) {
            return Err(RunvaneError::Config(format!(
                "tag key {k:?} given more than once"
            )));
        }
        map.insert(k.to_owned(), v.to_owned());
    }
    serde_json::to_vec(&map).map_err(|e| RunvaneError::Server(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_flag_language_is_validated() {
        let err = tags_to_json(&["lone-key".to_owned()]).unwrap_err();
        assert!(err.to_string().contains("key=value"));
        let ok = tags_to_json(&["env=prod".to_owned(), "zone=1".to_owned()]).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&ok).unwrap();
        assert_eq!(v["env"], "prod");
        assert_eq!(v["zone"], "1");
    }

    #[test]
    fn tags_reject_duplicate_keys_as_an_error_not_a_silent_last_win() {
        let err = tags_to_json(&["a=1".to_owned(), "a=2".to_owned()]).unwrap_err();
        assert!(matches!(err, RunvaneError::Config(_)));
        assert!(err.to_string().contains("more than once"));
    }

    #[test]
    fn json_rendering_roundtrips() {
        let raw = br#"{"a":[1,2,3]}"#.to_vec();
        assert_eq!(render_json(&raw).unwrap(), "{\n  \"a\": [\n    1,\n    2,\n    3\n  ]\n}");
        let list = vec![raw.clone(), raw];
        let out = render_json_list(&list).unwrap();
        assert_eq!(
            out,
            "[\n  {\n    \"a\": [\n      1,\n      2,\n      3\n    ]\n  },\n  {\n    \"a\": [\n      1,\n      2,\n      3\n    ]\n  }\n]"
        );
    }

    #[test]
    fn bad_json_is_a_typed_error() {
        let err = render_json(b"not json").unwrap_err();
        assert!(matches!(err, RunvaneError::Server(_)));
        let err2 = parse_json_bytes("nope", "input").unwrap_err();
        assert!(matches!(err2, RunvaneError::Config(_)));
    }
}