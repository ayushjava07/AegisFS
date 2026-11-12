//! gRPC mirror of the v1 HTTP surface.
//!
//! The generated stubs live behind [`proto`]. Handlers perform the same
//! conversions, validations, and store calls as the HTTP handlers in
//! [`crate::api::server`]; the mirror exists so tooling that prefers
//! protobuf-over-HTTP/2 can drive the platform without reimplementing the
//! contract. Elastic documents (run input, tags, metadata, and the responses
//! themselves) travel as serialized JSON.

// `tonic::Status` is a large type (it owns a metadata map); boxing each
// helper's error would buy nothing at these call sites.
#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;

use serde_json::Value as Json;
use tonic::{Request, Response, Status};

use crate::api::error::ApiError;
use crate::api::payloads::{TaskSpecPayload, WorkflowSpec};
use crate::api::server::AppState;
use crate::domain::ids::{RunId, generate_id};
use crate::domain::retry_policy::{BackoffKind, JitterKind, RetryPolicy};
use crate::domain::run::Run;
use crate::domain::status::{Priority, RunStatus};
use crate::domain::validation;
use crate::domain::workflow::{HookSpec, Hooks};
use crate::persistence::model::{ClaimToken, QueueEntry};
use crate::persistence::RunFilter;
use crate::state::run_fsm;

use self::proto::{
    runvane_server::Runvane, CancelRunRequest, GetRunRequest, GetWorkflowRequest,
    HealthRequest, HealthResponse, HookSpec as ProtoHookSpec, HooksSpec as ProtoHooksSpec,
    ListRunsRequest, ListRunsResponse, ListWorkflowsRequest, ListWorkflowsResponse, RunResponse,
    RetryPolicy as ProtoRetryPolicy, SubmitRunRequest, TaskSpec as ProtoTaskSpec,
    WorkflowResponse, WorkflowSpec as ProtoWorkflowSpec,
};

/// Generated protobuf stubs for `proto/runvane/v1/api.proto`.
#[allow(missing_docs)]
pub mod proto {
    tonic::include_proto!("runvane.v1");
}

/// Tonic service implementing the v1 control plane.
pub struct GrpcService {
    state: Arc<AppState>,
}

impl GrpcService {
    /// Wraps shared application state.
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }

    /// Returns the tonic server for the service (mirrors `build_router`).
    pub fn into_server(self) -> proto::runvane_server::RunvaneServer<Self> {
        proto::runvane_server::RunvaneServer::new(self)
    }
}

/// Maps any error that flattens into [`ApiError`] onto a tonic status with a
/// matching gRPC code.
fn to_tonic<E>(err: E) -> Status
where
    ApiError: From<E>,
{
    to_tonic_api(ApiError::from(err))
}

fn to_tonic_api(err: ApiError) -> Status {
    let code = match err.http_status() {
        400 => tonic::Code::InvalidArgument,
        404 => tonic::Code::NotFound,
        409 => tonic::Code::FailedPrecondition,
        502 => tonic::Code::Unavailable,
        _ => tonic::Code::Internal,
    };
    Status::new(code, err.message().to_owned())
}

fn parse_json(field: &str, bytes: &[u8]) -> Result<Json, Status> {
    if bytes.is_empty() {
        return Ok(Json::Null);
    }
    serde_json::from_slice(bytes)
        .map_err(|e| Status::invalid_argument(format!("{field} is not valid JSON: {e}")))
}

fn parse_tags(field: &str, bytes: &[u8]) -> Result<BTreeMap<String, String>, Status> {
    if bytes.is_empty() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_slice(bytes)
        .map_err(|e| Status::invalid_argument(format!("{field} is not an object of strings: {e}")))
}

fn priority_from(s: &str) -> Result<Priority, Status> {
    if s.is_empty() {
        return Ok(Priority::default());
    }
    Priority::from_str(s).map_err(|_| Status::invalid_argument(format!("invalid priority {s:?}")))
}

fn retry_from(proto: Option<&ProtoRetryPolicy>) -> Result<Option<RetryPolicy>, Status> {
    let Some(r) = proto else {
        return Ok(None);
    };
    // tonic omits unset fields: an all-zero policy means "absent".
    if r.max_attempts == 0
        && r.base_delay_ms == 0
        && r.max_delay_ms == 0
        && r.multiplier == 0.0
        && r.backoff.is_empty()
        && r.jitter.is_empty()
    {
        return Ok(None);
    }
    let backoff = match r.backoff.as_str() {
        "" | "fixed" => BackoffKind::Fixed,
        "linear" => BackoffKind::Linear,
        "exponential" => BackoffKind::Exponential,
        other => {
            return Err(Status::invalid_argument(format!(
                "invalid backoff {other:?}"
            )));
        }
    };
    let jitter = match r.jitter.as_str() {
        "" | "none" => JitterKind::None,
        "full" => JitterKind::Full,
        "equal" => JitterKind::Equal,
        other => return Err(Status::invalid_argument(format!("invalid jitter {other:?}"))),
    };
    let policy = RetryPolicy {
        max_attempts: r.max_attempts,
        base_delay_ms: r.base_delay_ms,
        max_delay_ms: r.max_delay_ms,
        multiplier: r.multiplier,
        backoff,
        jitter,
        retryable_only: r.retryable_only,
    };
    policy
        .validate()
        .map_err(|e| Status::invalid_argument(e.to_string()))?;
    Ok(Some(policy))
}

/// Maps the proto hooks block into the domain `Hooks` value.
fn hooks_from(proto: Option<&ProtoHooksSpec>) -> Result<Option<Hooks>, Status> {
    let Some(proto) = proto else {
        return Ok(None);
    };
    let mut hooks = Hooks::default();
    for spec in &proto.on_start {
        hooks.on_start.push(hook_from(spec)?);
    }
    for spec in &proto.on_success {
        hooks.on_success.push(hook_from(spec)?);
    }
    for spec in &proto.on_failure {
        hooks.on_failure.push(hook_from(spec)?);
    }
    for spec in &proto.on_cancel {
        hooks.on_cancel.push(hook_from(spec)?);
    }
    Ok(Some(hooks))
}

/// Maps a single wire hook spec into the domain type.
fn hook_from(proto: &ProtoHookSpec) -> Result<HookSpec, Status> {
    let spec = HookSpec {
        webhook_url: (!proto.webhook_url.is_empty()).then_some(proto.webhook_url.clone()),
        event_filter: (!proto.event_filter.is_empty()).then_some(proto.event_filter.clone()),
        headers: proto
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    };
    if let Some(url) = &spec.webhook_url {
        validation::validate_webhook_url(url).map_err(Status::invalid_argument)?;
    }
    Ok(spec)
}

fn task_from(proto: ProtoTaskSpec) -> Result<TaskSpecPayload, Status> {
    Ok(TaskSpecPayload {
        name: proto.name,
        handler: proto.handler,
        input: match parse_json("input", &proto.input_json)? {
            Json::Null => None,
            json => Some(json),
        },
        depends_on: if proto.depends_on.is_empty() {
            None
        } else {
            Some(proto.depends_on)
        },
        timeout_ms: (proto.timeout_ms != 0).then_some(proto.timeout_ms),
        retry: retry_from(proto.retry.as_ref())?,
        meta: if proto.meta_json.is_empty() {
            None
        } else {
            let meta: BTreeMap<String, Json> = serde_json::from_slice(&proto.meta_json)
                .map_err(|e| Status::invalid_argument(format!("meta is not an object: {e}")))?;
            if meta.is_empty() {
                None
            } else {
                Some(meta)
            }
        },
    })
}

#[tonic::async_trait]
impl Runvane for GrpcService {
    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(HealthResponse {
            status: "ok".to_owned(),
            booted_at_ms: self.state.boot_ms,
            now_ms: self.state.clock.now_ms(),
            queue_depth: self.state.store.len_queue() as i64,
            spec_version: 1,
        }))
    }

    async fn create_workflow(
        &self,
        request: Request<ProtoWorkflowSpec>,
    ) -> Result<Response<WorkflowResponse>, Status> {
        let wire = request.into_inner();
        let spec = WorkflowSpec {
            tenant: wire.tenant,
            name: wire.name,
            description: (!wire.description.is_empty()).then_some(wire.description),
            tasks: wire
                .tasks
                .into_iter()
                .map(task_from)
                .collect::<Result<Vec<_>, _>>()?,
            timeout_ms: (wire.timeout_ms != 0).then_some(wire.timeout_ms),
            retry: retry_from(wire.retry.as_ref())?,
            default_priority: Some(priority_from(&wire.default_priority)?),
            hooks: hooks_from(wire.hooks.as_ref())?,
            tags: Some(parse_tags("tags", &wire.tags_json)?),
        };
        let def = spec
            .into_definition(self.state.clock.now_ms())
            .map_err(to_tonic)?;
        let stored = self.state.store.put_workflow(def).map_err(to_tonic)?;
        let definition_json =
            serde_json::to_vec(&stored.def).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(WorkflowResponse { definition_json }))
    }

    async fn list_workflows(
        &self,
        request: Request<ListWorkflowsRequest>,
    ) -> Result<Response<ListWorkflowsResponse>, Status> {
        let tenant = request.into_inner().tenant;
        let recs = self.state.store.list_workflows().map_err(to_tonic)?;
        let mut definition_json = Vec::new();
        for rec in recs {
            if !tenant.is_empty() && rec.def.tenant != tenant {
                continue;
            }
            definition_json.push(
                serde_json::to_vec(&rec.def).map_err(|e| Status::internal(e.to_string()))?,
            );
        }
        Ok(Response::new(ListWorkflowsResponse { definition_json }))
    }

    async fn get_workflow(
        &self,
        request: Request<GetWorkflowRequest>,
    ) -> Result<Response<WorkflowResponse>, Status> {
        let req = request.into_inner();
        let rec = self
            .state
            .store
            .get_workflow(&req.tenant, &req.name)
            .map_err(to_tonic)?;
        let definition_json =
            serde_json::to_vec(&rec.def).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(WorkflowResponse { definition_json }))
    }

    async fn submit_run(
        &self,
        request: Request<SubmitRunRequest>,
    ) -> Result<Response<RunResponse>, Status> {
        let req = request.into_inner();
        let rec = self
            .state
            .store
            .get_workflow(&req.tenant, &req.name)
            .map_err(to_tonic)?;
        let input = match parse_json("input", &req.input_json)? {
            Json::Null => serde_json::json!({}),
            json => json,
        };
        validation::validate_run_input(&input).map_err(|e| to_tonic_api(ApiError::from(e)))?;
        let now_ms = self.state.clock.now_ms();
        let run_number = self
            .state
            .store
            .next_run_number(&req.name)
            .map_err(to_tonic)?;
        let run_id = RunId::from_validated(generate_id("rn_"));
        let run = Run {
            id: run_id.clone(),
            tenant: req.tenant,
            def_name: req.name,
            def_version: rec.def.version,
            input,
            status: RunStatus::Queued,
            attempts: 0,
            next_attempt_at_ms: None,
            deadline_at_ms: None,
            started_at_ms: None,
            finished_at_ms: None,
            error: None,
            output: None,
            tags: parse_tags("tags", &req.tags_json)?,
            created_at_ms: now_ms,
            run_number,
        };
        self.state.store.put_run(&run).map_err(to_tonic)?;
        self.state
            .store
            .enqueue(QueueEntry {
                run_id,
                token: ClaimToken::empty(),
                due_at_ms: now_ms,
                lease_until_ms: None,
                claimed_by: None,
            })
            .map_err(to_tonic)?;
        let run_json = serde_json::to_vec(&run).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(RunResponse { run_json }))
    }

    async fn list_runs(
        &self,
        request: Request<ListRunsRequest>,
    ) -> Result<Response<ListRunsResponse>, Status> {
        let req = request.into_inner();
        let status = match req.status.as_str() {
            "" => None,
            status => Some(
                RunStatus::from_str(status)
                    .map_err(|_| Status::invalid_argument(format!("invalid status {status:?}")))?,
            ),
        };
        let filter = RunFilter {
            tenant: (!req.tenant.is_empty()).then_some(req.tenant),
            name: (!req.name.is_empty()).then_some(req.name),
            status,
            limit: (req.limit > 0).then_some(req.limit as usize),
            ..RunFilter::default()
        };
        let runs = self.state.store.list_runs(&filter).map_err(to_tonic)?;
        let mut run_json = Vec::new();
        for run in runs {
            run_json.push(serde_json::to_vec(&run).map_err(|e| Status::internal(e.to_string()))?);
        }
        Ok(Response::new(ListRunsResponse { run_json }))
    }

    async fn get_run(
        &self,
        request: Request<GetRunRequest>,
    ) -> Result<Response<RunResponse>, Status> {
        let run_id = RunId::parse(&request.into_inner().id)
            .map_err(|e| to_tonic(crate::api::error::ApiError::from(crate::error::RunvaneError::from(e))))?;
        let run = self.state.store.get_run(&run_id).map_err(to_tonic)?;
        let run_json = serde_json::to_vec(&run).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(RunResponse { run_json }))
    }

    async fn cancel_run(
        &self,
        request: Request<CancelRunRequest>,
    ) -> Result<Response<RunResponse>, Status> {
        let run_id = RunId::parse(&request.into_inner().id).map_err(|e| {
            to_tonic(crate::api::error::ApiError::from(crate::error::RunvaneError::from(e)))
        })?;
        let current = self.state.store.get_run(&run_id).map_err(to_tonic)?;
        match current.status {
            RunStatus::Cancelled => {}
            status if status.is_terminal() => {
                return Err(Status::failed_precondition(format!(
                    "run {run_id} is already {status} and cannot be cancelled"
                )));
            }
            _ => {
                run_fsm::validate(current.status, RunStatus::Cancelled).map_err(to_tonic)?;
                self.state
                    .store
                    .cancel_run(&run_id, self.state.clock.now_ms())
                    .map_err(to_tonic)?;
            }
        }
        let run = self.state.store.get_run(&run_id).map_err(to_tonic)?;
        let run_json = serde_json::to_vec(&run).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(RunResponse { run_json }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::server::{AppState, build_router};
    use crate::clock::ManualClock;
    use crate::persistence::memory::MemoryStore;
    use crate::plugins::handler::Registry;

    use self::proto::runvane_client::RunvaneClient;
    use self::proto::{
        self as wire, GetRunRequest, TaskSpec as ProtoTaskSpec,
        WorkflowSpec as ProtoWorkflowSpec,
    };
    use tonic::transport::Server;

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState {
            store: Arc::new(MemoryStore::new()),
            registry: Arc::new(Registry::new()),
            clock: Arc::new(ManualClock::at(1_720_000_000_000)),
            boot_ms: 1_720_000_000_000,
        })
    }

    /// Binds a fresh loopback port, serves the service until the test is done.
    async fn spawn_server() -> (String, tokio::task::JoinHandle<()>) {
        // Learn an ephemeral port, then hand it to tonic's own bind. The
        // socket address is `Copy`, so no borrow leaks into the async task.
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let socket: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let service = GrpcService::new(test_state()).into_server();
        let handle = tokio::spawn(async move {
            Server::builder()
                .add_service(service)
                .serve(socket)
                .await
                .unwrap();
        });
        (format!("http://{socket}"), handle)
    }

    /// Returns a channel to the spawned server, retrying connect until the
    /// listener is accepting (bounded polling, so the suite never hangs).
    async fn connect(addr: &str) -> RunvaneClient<tonic::transport::Channel> {
        let mut last_error = None;
        for _ in 0..100 {
            match RunvaneClient::connect(addr.to_owned()).await {
                Ok(client) => return client,
                Err(err) => last_error = Some(err),
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("grpc server failed to come up on {addr}: {last_error:?}");
    }

    async fn client() -> (RunvaneClient<tonic::transport::Channel>, tokio::task::JoinHandle<()>) {
        let (addr, handle) = spawn_server().await;
        let client = connect(&addr).await;
        (client, handle)
    }

    fn spec() -> ProtoWorkflowSpec {
        ProtoWorkflowSpec {
            tenant: "acme".to_owned(),
            name: "ship".to_owned(),
            description: "release pipeline".to_owned(),
            tasks: vec![
                ProtoTaskSpec {
                    name: "build".to_owned(),
                    handler: "runvane.echo".to_owned(),
                    input_json: br#"{"step":"build"}"#.to_vec(),
                    depends_on: vec![],
                    timeout_ms: 0,
                    retry: None,
                    meta_json: vec![],
                },
                ProtoTaskSpec {
                    name: "deploy".to_owned(),
                    handler: "runvane.echo".to_owned(),
                    input_json: vec![],
                    depends_on: vec!["build".to_owned()],
                    timeout_ms: 0,
                    retry: None,
                    meta_json: vec![],
                },
            ],
            timeout_ms: 120_000,
            retry: None,
            default_priority: "normal".to_owned(),
            tags_json: br#"{"team":"infra"}"#.to_vec(),
            hooks: None,
        }
    }

    #[tokio::test]
    async fn health_round_trips() {
        let (mut client, handle) = client().await;
        let resp = client
            .health(wire::HealthRequest {})
            .await
            .unwrap()
            .into_inner();
        assert_eq!(resp.status, "ok");
        assert_eq!(resp.spec_version, 1);
        assert_eq!(resp.booted_at_ms, 1_720_000_000_000);
        assert_eq!(resp.queue_depth, 0);
        handle.abort();
    }

    #[tokio::test]
    async fn create_submit_list_get_cancel_round_trip() {
        let (mut client, handle) = client().await;

        // Create + fetch.
        let created = client
            .create_workflow(spec())
            .await
            .unwrap()
            .into_inner();
        let def: crate::domain::workflow::WorkflowDef =
            serde_json::from_slice(&created.definition_json).unwrap();
        assert_eq!(def.name, "ship");
        assert_eq!(def.tasks.len(), 2);

        let fetched = client
            .get_workflow(wire::GetWorkflowRequest {
                tenant: "acme".to_owned(),
                name: "ship".to_owned(),
            })
            .await
            .unwrap()
            .into_inner();
        assert!(!fetched.definition_json.is_empty());

        // Submit + fetch run.
        let submitted = client
            .submit_run(wire::SubmitRunRequest {
                tenant: "acme".to_owned(),
                name: "ship".to_owned(),
                input_json: br#"{"ref":"main"}"#.to_vec(),
                tags_json: br#"{"env":"prod"}"#.to_vec(),
            })
            .await
            .unwrap()
            .into_inner();
        let run: Run = serde_json::from_slice(&submitted.run_json).unwrap();
        assert_eq!(run.status, RunStatus::Queued);
        assert_eq!(run.run_number, 1);
        assert!(run.id.as_str().starts_with("rn_"));

        let got = client
            .get_run(GetRunRequest {
                id: run.id.as_str().to_owned(),
            })
            .await
            .unwrap()
            .into_inner();
        let got_run: Run = serde_json::from_slice(&got.run_json).unwrap();
        assert_eq!(got_run.id, run.id);
        assert_eq!(got_run.tags.get("env"), Some(&"prod".to_owned()));

        // Cancel + idempotent re-cancel.
        let cancelled = client
            .cancel_run(wire::CancelRunRequest {
                id: run.id.as_str().to_owned(),
            })
            .await
            .unwrap()
            .into_inner();
        let cancelled_run: Run = serde_json::from_slice(&cancelled.run_json).unwrap();
        assert_eq!(cancelled_run.status, RunStatus::Cancelled);

        let again = client
            .cancel_run(wire::CancelRunRequest {
                id: run.id.as_str().to_owned(),
            })
            .await
            .unwrap()
            .into_inner();
        let again_run: Run = serde_json::from_slice(&again.run_json).unwrap();
        assert_eq!(again_run.status, RunStatus::Cancelled);

        // Health reflects the dequeue.
        let health = client
            .health(wire::HealthRequest {})
            .await
            .unwrap()
            .into_inner();
        assert_eq!(health.queue_depth, 0);

        handle.abort();
    }

    #[tokio::test]
    async fn hooks_round_trip_and_validation() {
        let (mut client, handle) = client().await;

        // A definition with a lifecycle hook round-trips into the domain doc.
        let spec = spec();
        let mut with_hooks = spec.clone();
        with_hooks.hooks = Some(wire::HooksSpec {
            on_success: vec![wire::HookSpec {
                webhook_url: "http://127.0.0.1:1/hook".to_owned(),
                event_filter: "run.succeeded".to_owned(),
                headers: std::collections::HashMap::from([(
                    "X-Team".to_owned(),
                    "infra".to_owned(),
                )]),
            }],
            ..wire::HooksSpec::default()
        });
        let created = client
            .create_workflow(with_hooks.clone())
            .await
            .unwrap()
            .into_inner();
        let def: crate::domain::workflow::WorkflowDef =
            serde_json::from_slice(&created.definition_json).unwrap();
        assert_eq!(def.hooks.on_success.len(), 1);
        assert_eq!(
            def.hooks.on_success[0].webhook_url.as_deref(),
            Some("http://127.0.0.1:1/hook")
        );
        assert_eq!(
            def.hooks.on_success[0].headers.get("X-Team"),
            Some(&"infra".to_owned())
        );

        // Invalid hook URL is rejected before the definition is stored.
        let mut bad = spec.clone();
        bad.hooks = Some(wire::HooksSpec {
            on_start: vec![wire::HookSpec {
                webhook_url: "not a url".to_owned(),
                ..wire::HookSpec::default()
            }],
            ..wire::HooksSpec::default()
        });
        let err = client.create_workflow(bad).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("webhook url"));

        handle.abort();
    }

    #[tokio::test]
    async fn error_codes_map_to_tonic_statuses() {
        let (mut client, handle) = client().await;

        // Unknown workflow submit.
        let err = client
            .submit_run(wire::SubmitRunRequest {
                tenant: "acme".to_owned(),
                name: "missing".to_owned(),
                input_json: vec![],
                tags_json: vec![],
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);

        // Malformed run id.
        let err = client
            .get_run(GetRunRequest {
                id: "not-an-id".to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);

        // Duplicate task names rejected at create.
        let mut dup = spec();
        dup.tasks[0].name = "same".to_owned();
        dup.tasks[1].name = "same".to_owned();
        let err = client.create_workflow(dup).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);

        handle.abort();
    }

    #[test]
    fn state_coerces_to_both_servers() {
        let state = test_state();
        let _http = build_router(state.clone());
        let _grpc = GrpcService::new(state).into_server();
    }
}