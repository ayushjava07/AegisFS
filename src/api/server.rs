//! HTTP server: shared state, router, and handlers for the v1 surface.
//!
//! Handlers are intentionally thin compositors over the store, the clock, and
//! the payload converters. Every non-trivial path is exercised end-to-end by
//! the router tests at the bottom of this file (real MemoryStore, real
//! Registry, manual clock) so the wire contract — status codes, envelope
//! shape, error bodies — is pinned without spawning a listener.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::clock::Clock;
use crate::domain::ids::{generate_id, RunId};
use crate::domain::run::Run;
use crate::domain::status::RunStatus;
use crate::domain::validation;
use crate::domain::workflow::WorkflowDef;
use crate::persistence::model::{ClaimToken, QueueEntry};
use crate::persistence::Store;
use crate::plugins::handler::Registry;
use crate::state::run_fsm;

use super::error::{api_err, ApiError};
use super::payloads::{Envelope, HealthView, RunQuery, SubmitRunRequest, WorkflowSpec};

/// Shared state handed to every handler.
#[derive(Clone)]
pub struct AppState {
    /// The durable store (memory or SQLite) behind all read/write paths.
    pub store: Arc<dyn Store>,
    /// Handler registry used by the scheduler; surfaced for introspection.
    pub registry: Arc<Registry>,
    /// Platform clock; the single source of truth for timestamps.
    pub clock: Arc<dyn Clock>,
    /// Server boot timestamp, captured once at construction.
    pub boot_ms: i64,
    /// Process-level metric counters for the debug page.
    pub metrics: Arc<crate::telemetry::Metrics>,
}

/// Builds the v1 router wired to `state`.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/v1/health", get(health))
        .route("/v1/workflows", get(list_workflows).post(create_workflow))
        .route("/v1/workflows/:tenant/:name", get(get_workflow))
        .route("/v1/workflows/:tenant/:name/runs", post(submit_run))
        .route("/v1/runs", get(list_runs))
        .route("/v1/runs/:id", get(get_run))
        .route("/v1/runs/:id/cancel", post(cancel_run))
        .route("/v1/debug/metrics", get(metrics))
        .with_state(state)
}

/// `GET /` — a minimal read-only overview rendered inline (no assets).
async fn dashboard(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use std::fmt::Write;

    let runs = state
        .store
        .list_runs(&crate::persistence::RunFilter::default())
        .unwrap_or_default();
    let summaries = state.store.list_workflow_summaries().unwrap_or_default();
    let now_ms = state.clock.now_ms();
    let uptime_s = (now_ms - state.boot_ms).max(0) / 1000;

    let mut html = String::new();
    let _ = write!(html, "<!doctype html><html><head><title>runvane</title>");
    let _ = write!(
        html,
        "<style>body{{font:14px/1.5 -apple-system,sans-serif;max-width:960px;margin:2rem auto;padding:0 1rem}}table{{border-collapse:collapse;width:100%}}td,th{{border:1px solid #ddd;padding:.4rem;text-align:left}}</style></head><body>"
    );
    let _ = write!(
        html,
        "<h1>runvane</h1><p>v{} &middot; uptime {}s &middot; queue depth {}</p>",
        crate::telemetry::VERSION,
        uptime_s,
        state.store.len_queue(),
    );
    let _ = write!(
        html,
        "<section><h2>workflows (run count)</h2><table><tr><th>name</th><th>runs</th></tr>"
    );
    for summary in summaries {
        let _ = write!(
            html,
            "<tr><td><code>{}/{}</code></td><td>{}</td></tr>",
            summary.workflow.def.tenant, summary.workflow.def.name, summary.run_count
        );
    }
    let _ = write!(html, "</table></section>");
    let _ = write!(
        html,
        "<section><h2>recent runs</h2><table><tr><th>id</th><th>tenant/def</th><th>status</th><th>finished</th></tr>"
    );
    for run in runs {
        let finished = run.finished_at_ms.unwrap_or(run.created_at_ms);
        let _ = write!(
            html,
            "<tr><td><code>{}</code></td><td>{}/{}</td><td>{}</td><td>{}</td></tr>",
            run.id.as_str(),
            run.tenant,
            run.def_name,
            run.status,
            finished,
        );
    }
    let _ = write!(html, "</table></section>");
    let _ = write!(
        html,
        "<p><a href=\"/v1/debug/metrics\">metrics</a></p></body></html>"
    );
    (
        axum::http::StatusCode::OK,
        [("content-type", "text/html; charset=utf-8")],
        html,
    )
}

/// `GET /v1/health` — liveness and coarse queue depth.
async fn health(State(state): State<Arc<AppState>>) -> Json<Envelope<HealthView>> {
    Json(Envelope::of(HealthView {
        status: "ok",
        booted_at_ms: state.boot_ms,
        now_ms: state.clock.now_ms(),
        queue_depth: state.store.len_queue(),
        spec_version: 1,
    }))
}

/// `GET /v1/debug/metrics` — counter snapshot plus store-derived gauges.
async fn metrics(State(state): State<Arc<AppState>>) -> Json<MetricsView> {
    let snapshot = state.metrics.snapshot();
    let gauges = Gauges {
        queue_depth: state.store.len_queue() as u64,
        workflows: state.store.list_workflows().map_or(0, |r| r.len()),
        runs: state
            .store
            .list_runs(&crate::persistence::RunFilter::default())
            .map_or(0, |r| r.len()),
    };
    Json(MetricsView { snapshot, gauges })
}

/// Envelope variant only for the metrics page: counters + gauges side by side.
#[derive(Debug, serde::Serialize)]
struct MetricsView {
    snapshot: crate::telemetry::MetricsSnapshot,
    gauges: Gauges,
}

/// Store-derived gauges computed on demand.
#[derive(Debug, serde::Serialize)]
struct Gauges {
    queue_depth: u64,
    workflows: usize,
    runs: usize,
}

/// `POST /v1/workflows` — create a definition.
async fn create_workflow(
    State(state): State<Arc<AppState>>,
    Json(spec): Json<WorkflowSpec>,
) -> Result<impl IntoResponse, ApiError> {
    let def = spec
        .into_definition(state.clock.now_ms())
        .map_err(api_err)?;
    let stored = state.store.put_workflow(def).map_err(ApiError::from)?;
    state
        .metrics
        .workflow_creations_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok((StatusCode::CREATED, Json(Envelope::of(stored.def))))
}

/// `GET /v1/workflows` — list all definitions.
async fn list_workflows(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Envelope<Vec<WorkflowDef>>>, ApiError> {
    let recs = state.store.list_workflows().map_err(ApiError::from)?;
    Ok(Json(Envelope::of(
        recs.into_iter().map(|rec| rec.def).collect(),
    )))
}

/// `GET /v1/workflows/{tenant}/{name}` — fetch one definition.
async fn get_workflow(
    State(state): State<Arc<AppState>>,
    Path((tenant, name)): Path<(String, String)>,
) -> Result<Json<Envelope<WorkflowDef>>, ApiError> {
    let rec = state
        .store
        .get_workflow(&tenant, &name)
        .map_err(ApiError::from)?;
    Ok(Json(Envelope::of(rec.def)))
}

/// `POST /v1/workflows/{tenant}/{name}/runs` — submit a run.
async fn submit_run(
    State(state): State<Arc<AppState>>,
    Path((tenant, name)): Path<(String, String)>,
    Json(body): Json<SubmitRunRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let rec = state
        .store
        .get_workflow(&tenant, &name)
        .map_err(ApiError::from)?;
    let input = body.input.unwrap_or_else(|| serde_json::json!({}));
    validation::validate_run_input(&input).map_err(api_err)?;
    let now_ms = state.clock.now_ms();
    let run_number = state.store.next_run_number(&name).map_err(ApiError::from)?;
    let id = RunId::from_validated(generate_id("rn_"));
    let run = Run {
        id: id.clone(),
        tenant,
        def_name: name.clone(),
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
        tags: body.tags.unwrap_or_default(),
        created_at_ms: now_ms,
        run_number,
    };
    state.store.put_run(&run).map_err(ApiError::from)?;
    state
        .store
        .enqueue(QueueEntry {
            run_id: id,
            token: ClaimToken::empty(),
            due_at_ms: now_ms,
            lease_until_ms: None,
            claimed_by: None,
        })
        .map_err(ApiError::from)?;
    state
        .metrics
        .run_submissions_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok((StatusCode::ACCEPTED, Json(Envelope::of(run))))
}

/// `GET /v1/runs` — query runs by definition, status, and page size.
async fn list_runs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<RunQuery>,
) -> Result<Json<Envelope<Vec<Run>>>, ApiError> {
    let filter = query.into_filter()?;
    let runs = state.store.list_runs(&filter).map_err(ApiError::from)?;
    Ok(Json(Envelope::of(runs)))
}

/// `GET /v1/runs/{id}` — fetch one run.
async fn get_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Envelope<Run>>, ApiError> {
    let run_id = RunId::parse(&id).map_err(api_err)?;
    let run = state.store.get_run(&run_id).map_err(ApiError::from)?;
    Ok(Json(Envelope::of(run)))
}

/// `POST /v1/runs/{id}/cancel` — operator cancellation, best-effort.
async fn cancel_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Envelope<Run>>, ApiError> {
    let run_id = RunId::parse(&id).map_err(api_err)?;
    let current = state.store.get_run(&run_id).map_err(ApiError::from)?;
    match current.status {
        // Idempotent: an already-cancelled run returns as-is.
        RunStatus::Cancelled => return Ok(Json(Envelope::of(current))),
        status if status.is_terminal() => {
            return Err(ApiError::conflict(format!(
                "run {id} is already {status} and cannot be cancelled"
            )));
        }
        _ => {}
    }
    run_fsm::validate(current.status, RunStatus::Cancelled).map_err(api_err)?;
    state
        .store
        .cancel_run(&run_id, state.clock.now_ms())
        .map_err(ApiError::from)?;
    state
        .metrics
        .run_cancellations_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let updated = state.store.get_run(&run_id).map_err(ApiError::from)?;
    Ok(Json(Envelope::of(updated)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as HttpStatus};
    use axum::response::Response;
    use serde_json::{json, Value};
    use tower::util::ServiceExt;

    use crate::clock::ManualClock;
    use crate::persistence::memory::MemoryStore;

    fn test_state() -> Arc<AppState> {
        let clock = ManualClock::at(1_720_000_000_000);
        Arc::new(AppState {
            store: Arc::new(MemoryStore::new()),
            registry: Arc::new(Registry::new()),
            clock: Arc::new(clock),
            boot_ms: 1_720_000_000_000,
            metrics: crate::telemetry::shared(),
        })
    }

    async fn send(app: &Router, req: Request<Body>) -> Response {
        app.clone().oneshot(req).await.unwrap()
    }

    fn json_body(value: &Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .header("content-type", "application/json")
            .uri("/v1/workflows")
            .body(Body::from(value.to_string()))
            .unwrap()
    }

    async fn read_json(res: Response) -> Value {
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn sample_spec() -> Value {
        json!({
            "tenant": "acme",
            "name": "ship",
            "description": "release pipeline",
            "tasks": [
                { "name": "build", "handler": "runvane.echo", "input": { "step": "build" } },
                { "name": "deploy", "handler": "runvane.echo", "dependsOn": ["build"] }
            ],
            "timeoutMs": 120_000
        })
    }

    async fn seed_workflow(app: &Router, spec: &Value) -> Value {
        let res = send(app, json_body(spec)).await;
        assert_eq!(res.status(), HttpStatus::CREATED);
        read_json(res).await
    }

    #[tokio::test]
    async fn health_reports_ok_and_queue_depth() {
        let app = build_router(test_state());
        let res = send(
            &app,
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::OK);
        let body = read_json(res).await;
        assert_eq!(body["specVersion"], 1);
        assert_eq!(body["data"]["status"], "ok");
        assert_eq!(body["data"]["queueDepth"], 0);
    }

    #[tokio::test]
    async fn create_workflow_stamps_id_version_and_defaults() {
        let app = build_router(test_state());
        let body = seed_workflow(&app, &sample_spec()).await;
        let def = &body["data"];
        assert!(def["id"].as_str().unwrap().starts_with("wf_"));
        assert_eq!(def["name"], "ship");
        assert_eq!(def["tenant"], "acme");
        assert_eq!(def["version"], 1);
        assert_eq!(def["spec_version"], 1);
        // Defaulted fields survive the round trip.
        assert_eq!(def["default_priority"], "normal");
        assert_eq!(def["timeout_ms"], 120_000);
        let tasks = def["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[1]["handler"], "runvane.echo");
    }

    #[tokio::test]
    async fn create_workflow_rejects_invalid_definitions() {
        let app = build_router(test_state());
        // Empty task list.
        let bare = json!({ "tenant": "acme", "name": "ship", "tasks": [] });
        let res = send(&app, json_body(&bare)).await;
        assert_eq!(res.status(), HttpStatus::BAD_REQUEST);
        let body = read_json(res).await;
        assert_eq!(body["error"]["code"], "validation");

        // Duplicate task names are rejected by definition validation.
        let dup_names = json!({
            "tenant": "acme",
            "name": "dups",
            "tasks": [
                { "name": "t", "handler": "runvane.echo" },
                { "name": "t", "handler": "runvane.echo" }
            ]
        });
        let res = send(&app, json_body(&dup_names)).await;
        assert_eq!(res.status(), HttpStatus::BAD_REQUEST);

        // State stayed clean.
        let res = send(
            &app,
            Request::builder()
                .uri("/v1/workflows")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        let body = read_json(res).await;
        assert_eq!(body["data"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn get_workflow_happy_and_missing() {
        let app = build_router(test_state());
        seed_workflow(&app, &sample_spec()).await;

        let res = send(
            &app,
            Request::builder()
                .uri("/v1/workflows/acme/ship")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::OK);
        let body = read_json(res).await;
        assert_eq!(body["data"]["name"], "ship");

        let res = send(
            &app,
            Request::builder()
                .uri("/v1/workflows/acme/missing")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::NOT_FOUND);
        let body = read_json(res).await;
        assert_eq!(body["error"]["code"], "not_found");
    }

    #[tokio::test]
    async fn submit_run_queues_and_increments_counter() {
        let app = build_router(test_state());
        seed_workflow(&app, &sample_spec()).await;

        let submit = json!({ "input": { "ref": "main" }, "tags": { "env": "prod" } });
        let res = send(
            &app,
            Request::builder()
                .method("POST")
                .header("content-type", "application/json")
                .uri("/v1/workflows/acme/ship/runs")
                .body(Body::from(submit.to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::ACCEPTED);
        let body = read_json(res).await;
        assert_eq!(body["data"]["status"], "queued");
        assert_eq!(body["data"]["run_number"], 1);
        let run_id = body["data"]["id"].as_str().unwrap().to_owned();
        assert!(run_id.starts_with("rn_"));

        let second = send(
            &app,
            Request::builder()
                .method("POST")
                .header("content-type", "application/json")
                .uri("/v1/workflows/acme/ship/runs")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
        assert_eq!(second.status(), HttpStatus::ACCEPTED);
        let body = read_json(second).await;
        assert_eq!(body["data"]["run_number"], 2);

        // Both runs visible via list, with the submit filter.
        let res = send(
            &app,
            Request::builder()
                .uri("/v1/runs?tenant=acme&name=ship&status=queued")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        let body = read_json(res).await;
        assert_eq!(body["data"].as_array().unwrap().len(), 2);

        // Queue depth reflects the two pending submissions.
        let res = send(
            &app,
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        let body = read_json(res).await;
        assert_eq!(body["data"]["queueDepth"], 2);
    }

    #[tokio::test]
    async fn submit_run_requires_known_workflow_and_small_input() {
        let app = build_router(test_state());
        seed_workflow(&app, &sample_spec()).await;

        // Unknown definition.
        let res = send(
            &app,
            Request::builder()
                .method("POST")
                .header("content-type", "application/json")
                .uri("/v1/workflows/acme/other/runs")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::NOT_FOUND);

        // Oversized input is rejected by the size boundary validator.
        let blob = "x".repeat(1_048_577);
        let big = json!({ "input": { "blob": blob } });
        let res = send(
            &app,
            Request::builder()
                .method("POST")
                .header("content-type", "application/json")
                .uri("/v1/workflows/acme/ship/runs")
                .body(Body::from(big.to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_run_round_trips_submitted_run() {
        let app = build_router(test_state());
        seed_workflow(&app, &sample_spec()).await;

        let res = send(
            &app,
            Request::builder()
                .method("POST")
                .header("content-type", "application/json")
                .uri("/v1/workflows/acme/ship/runs")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
        let submitted = read_json(res).await;
        let run_id = submitted["data"]["id"].as_str().unwrap();

        let res = send(
            &app,
            Request::builder()
                .uri(format!("/v1/runs/{run_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::OK);
        let body = read_json(res).await;
        assert_eq!(body["data"]["id"], run_id);
        assert_eq!(body["data"]["def_name"], "ship");
        assert_eq!(body["data"]["def_version"], 1);

        let res = send(
            &app,
            Request::builder()
                .uri("/v1/runs/rn_nope123")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_runs_filters_and_caps_limit() {
        let app = build_router(test_state());
        seed_workflow(&app, &sample_spec()).await;
        for _ in 0..3 {
            let res = send(
                &app,
                Request::builder()
                    .method("POST")
                    .header("content-type", "application/json")
                    .uri("/v1/workflows/acme/ship/runs")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await;
            assert_eq!(res.status(), HttpStatus::ACCEPTED);
        }

        let res = send(
            &app,
            Request::builder()
                .uri("/v1/runs?limit=2")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        let body = read_json(res).await;
        assert_eq!(body["data"].as_array().unwrap().len(), 2);

        let res = send(
            &app,
            Request::builder()
                .uri("/v1/runs?status=running")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        let body = read_json(res).await;
        assert_eq!(body["data"].as_array().unwrap().len(), 0);

        let res = send(
            &app,
            Request::builder()
                .uri("/v1/runs?status=banana")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::BAD_REQUEST);

        let res = send(
            &app,
            Request::builder()
                .uri("/v1/runs?limit=0")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::BAD_REQUEST);
    }

    #[tokio::test]
    async fn cancel_queued_run_transitions_and_idempotency() {
        let app = build_router(test_state());
        seed_workflow(&app, &sample_spec()).await;

        let res = send(
            &app,
            Request::builder()
                .method("POST")
                .header("content-type", "application/json")
                .uri("/v1/workflows/acme/ship/runs")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
        let run_id = read_json(res).await["data"]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        let res = send(
            &app,
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{run_id}/cancel"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::OK);
        let body = read_json(res).await;
        assert_eq!(body["data"]["status"], "cancelled");

        // Second cancel is idempotent-success.
        let res = send(
            &app,
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{run_id}/cancel"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::OK);

        // Queue entry removed: health no longer reports it pending.
        let res = send(
            &app,
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        let body = read_json(res).await;
        assert_eq!(body["data"]["queueDepth"], 0);
    }

    #[tokio::test]
    async fn dashboard_renders_overview_with_rows() {
        let app = build_router(test_state());

        let create = json_body(&serde_json::json!({
            "tenant": "acme",
            "name": "nightly",
            "tasks": [{"name": "a", "handler": "runvane.echo"}],
        }));
        let res = send(&app, create).await;
        assert_eq!(res.status(), HttpStatus::CREATED);

        let res = send(
            &app,
            Request::builder().uri("/").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::OK);
        let headers = res.headers();
        assert_eq!(
            headers.get("content-type").unwrap().to_str().unwrap(),
            "text/html; charset=utf-8"
        );
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("acme/nightly"), "workflow row rendered");
        assert!(text.contains("queue depth"), "overview header rendered");
        assert!(text.contains("/v1/debug/metrics"), "metrics link present");
    }

    #[tokio::test]
    async fn debug_metrics_reports_counters_and_gauges() {
        let app = build_router(test_state());

        // Plant a workflow and a run so the gauges are non-trivial.
        let create = json_body(&serde_json::json!({
            "tenant": "acme",
            "name": "ship",
            "tasks": [{"name": "a", "handler": "runvane.echo"}],
        }));
        let res = send(&app, create).await;
        assert_eq!(res.status(), HttpStatus::CREATED);
        let res = send(
            &app,
            Request::builder()
                .method("POST")
                .header("content-type", "application/json")
                .uri("/v1/workflows/acme/ship/runs")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::ACCEPTED);

        let res = send(
            &app,
            Request::builder()
                .uri("/v1/debug/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::OK);
        let body = read_json(res).await;
        assert_eq!(body["snapshot"]["workflow_creations_total"], 1);
        assert_eq!(body["snapshot"]["run_submissions_total"], 1);
        assert_eq!(body["gauges"]["queue_depth"], 1);
        assert_eq!(body["gauges"]["runs"], 1);
        assert!(!body["snapshot"]["version"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancel_missing_and_malformed_ids_surface_proper_codes() {
        let app = build_router(test_state());

        let res = send(
            &app,
            Request::builder()
                .method("POST")
                .uri("/v1/runs/rn_nope123/cancel")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::NOT_FOUND);

        let res = send(
            &app,
            Request::builder()
                .method("POST")
                .uri("/v1/runs/not-an-id/cancel")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status(), HttpStatus::BAD_REQUEST);
    }
}
