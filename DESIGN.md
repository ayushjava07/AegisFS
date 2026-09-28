# Runvane — Design Document

## Product

**Runvane** is an internal, self-hosted distributed workflow-orchestration
platform aimed at platform/back-end engineering teams. Engineers declare
durable, retryable workflows as versioned definition documents, submit runs
over HTTP or gRPC, and observe execution through the scheduler, the webhooks,
and the read-only status dashboard. Execution is delegated to pluggable task
handlers shipped as first-party plugins, so the core stays transport- and
handler-agnostic.

The name "Runvane" is an invented portmanteau of "run" and "weather vane":
it orients the operator toward what a run is doing, the same way a vane shows
the direction of the wind. The project is developed here under the existing
`aegisfs` repository name for historical reasons; from Phase 0 onward the
crate, binary, and module tree are the Runvane platform.

### Pitch

- Durable run state backed by a pluggable store (in-memory for tests, an
  embedded SQL database for single-node production) behind one `Store` trait.
- A small, verified state machine with explicit, table-driven legal
  transitions for runs and tasks.
- Retries with exponential backoff, decoupled jitter, and per-task timeouts,
  all driven through a `Clock` abstraction so behavior is deterministic under
  test.
- Two first-party task-handler plugins (HTTP call-out and shell command)
  against a small `TaskHandler` trait.
- HTTP (JSON) and gRPC (Protobuf) API surfaces backed by the same services.
- Operator CLI for configuration, submission, inspection, and admin actions.
- Metrics, structured tracing, and a minimal server-rendered status dashboard.

### Non-goals

- Multi-node consensus/coordination (single-node durable queue for now; the
  store trait and queue semantics leave room for a distributed leader later).
- A general-purpose scripting language for workflows (steps reference
  handlers, they are not programs).
- Client SDKs in other languages; first contact is the HTTP/gRPC surface.

## Module layout

Runvane is a single Cargo package (`runvane`) exposing both a binary
(`src/main.rs`) and a library (`src/lib.rs`). Subsystems map one-to-one onto
modules so the defect catalogue in Phase 10 has a natural home for every
category.

```
src/
  lib.rs             crate root, module registry, prelude
  main.rs            binary entrypoint; wires config -> store -> services
  domain/            core domain types and validation
    mod.rs
    ids.rs           typed identifiers
    workflow.rs      workflow definitions and task specs
    run.rs           run and task-run records
    retry_policy.rs  retry/backoff configuration
    hooks.rs         completion hook specifications
    status.rs        status enumerations
    validation.rs    structural validation of definitions/inputs
    error.rs         domain error types
  state/             state-machine engine
    mod.rs
    machine.rs       generic transition engine
    run_fsm.rs       workflow-run state machine
    task_fsm.rs      task-run state machine
    invariant.rs     reachable-state / invariant checks
    snapshots.rs     state snapshotting utilities
  clock/             time abstraction for determinism
    mod.rs           Clock trait, SystemClock, ManualClock, TimeSource
  persistence/       store trait + backends + migrations
    mod.rs           Store trait and DataError
    model.rs         wire records shared by backends
    memory.rs        in-memory backend
    sqlite.rs        embedded SQL backend
    migrations.rs    schema management
    filter.rs        run/task listing filters and pagination
  queue/             durable run queue
    mod.rs           Queue, QueueEntry, Priority, ClaimResult
    memory_queue.rs  in-memory queue implementation
    sql_queue.rs     SQL queue implementation
    lease.rs         lease/claim semantics
  scheduler/         scheduler and worker pool
    mod.rs           Scheduler, WorkerPool, drain semantics
    pool.rs          worker pool
    pump.rs          queue -> ready-runs pump loop
    run_sup.rs       run supervisor orchestration
    tasks.rs         task-dispatch/retry planning
  retry/             retry/backoff policy engine
    mod.rs           RetryState, backoff calculators, jitter
    backoff.rs       exponential, linear, fixed, full-jitter
    planner.rs       next-attempt planning and deadline math
    error.rs
  handler/           task-handler trait and adapters
    mod.rs           TaskHandler, HandlerRegistry, HandlerSpec
    registry.rs      registry with dispatch
    context.rs       HandlerContext, HandlerResult, artifacts
  plugins/           first-party plugins
    mod.rs
    http_call.rs     HTTP-plugin task handler
    shell.rs         shell command task handler
    echo.rs          deterministic echo plugin (tests/demo)
    stub.rs          stubbed failure plugin for tests/demo
  cache/             definition cache with explicit invalidation
    mod.rs           DefinitionCache, CacheStats, EvictionPolicy
    lru.rs           LRU + generational invalidation
  api/               HTTP API
    mod.rs           router construction
    http/            axum wiring (server, middleware, router)
    routes_workflow.rs
    routes_runs.rs
    routes_admin.rs
    routes_meta.rs   health, debug, metrics
    payloads.rs      request/response DTOs + versioning
    mapper.rs        domain <-> wire conversion
    error.rs         HTTP error mapping
    middleware.rs    auth/RBAC/request-id/logging
  rpc/               gRPC API
    mod.rs
    proto/           .proto files (build.rs compiles)
    service.rs       tonic service implementations
    convert.rs       domain <-> protobuf mapping
  cli/               operator CLI
    mod.rs           clap definition
    submit.rs        run submission
    inspect.rs       run inspection
    admin.rs         admin actions
    plugins.rs       plugin listing
    config_cmd.rs    config validate/show
    console.rs       terminal rendering
  config/            configuration layer
    mod.rs           Config, Sources, Precedence
    files.rs         file loaders (JSON/TOML)
    env.rs           env-overlay
    defaults.rs      structural defaults
    validate.rs      config validation
  event/             event bus and webhooks
    mod.rs           Event, EventKind, dispatch
    bus.rs           in-process subscriber bus (bounded)
    webhook.rs       webhook delivery with retry + HMAC
    sign.rs          HMAC-SHA256 signing/verification
    payload.rs       event payload serialization
  telemetry/         metrics and observability
    mod.rs           Metrics registry
    metrics.rs       Counter/Gauge/Histogram
    registry.rs
    format.rs        prometheus text exposition
    trace.rs         tracing subscriber wiring + events
  maintenance/       background workers
    mod.rs           MaintenanceScheduler
    expirer.rs       old-run expiry
    compactor.rs     history compaction
    stats.rs         periodic stats snapshots
  auth/              API token auth + RBAC
    mod.rs
    token.rs         token generation/verification
    rbac.rs          roles, permissions, policy evaluation
    store.rs         token store (memory + sql)
  dashboard/         read-only HTML status dashboard
    mod.rs           routing for /dashboard
    render.rs        HTML rendering helpers (escaping, tables)
    pages.rs         overview, run detail, workflow listing
  audit/             audit log
    mod.rs           AuditEvent, recorder
  util/              small shared helpers
    mod.rs
    idgen.rs
    json.rs          tolerant JSON value helpers
    text.rs          truncation/escaping
    result.rs        result combinators
  error.rs           top-level error + context
```

## Subsystem boundaries and responsibilities

| Subsystem | Module(s) | Owns |
|---|---|---|
| Workflow/task definition & state machine | `domain`, `state` | types, validation, legal transitions |
| Scheduler + worker pool | `scheduler`, `queue` | claim, dispatch, supervision, draining |
| Persistence | `persistence` | store trait, in-memory + SQL backends, migrations |
| Definition cache | `cache` | hot-def caching, TTL, explicit invalidation |
| HTTP API | `api` | versioned JSON surface |
| gRPC API | `rpc` | protobuf surface mirroring HTTP subset |
| CLI | `cli` | operator tooling |
| Plugin/extension interface | `handler`, `plugins` | task-handler trait + first-party plugins |
| Retry/backoff/timeout | `retry`, `clock` | backoff math, next-attempt planning, deadlines |
| Webhook/event dispatch | `event` | events, subscriber bus, webhook delivery |
| Metrics/observability | `telemetry` | counters/histograms, `/debug`, `/metrics` |
| Configuration | `config` | file+env+flag merging, precedence |
| Auth/RBAC | `auth` | tokens, roles, policy |
| Maintenance workers | `maintenance` | expiry, compaction |
| Status dashboard | `dashboard` | read-only HTML pages |
| Migrations/fixtures | `persistence::migrations`, `persistence::fixtures` | schema + seed data |

The scheduler and the maintenance workers are the only components that run
long-lived background tasks; everything else front-loads on the scheduler's
lifecycle so `Scheduler::drain` covers all close paths.

## Data model sketch

```
WorkflowDef {
  id: WorkflowId        // urn-ish typed id, e.g. wf_<base32>
  tenant: TenantId
  name: String          // unique per tenant
  version: u32          // bump on redefine; cache key is (tenant,name,version)
  description: String
  tasks: Vec<TaskSpec>  // unordered list; ordering via depends_on
  timeout: Duration     // whole-workflow deadline from first dispatch
  retry: Option<RetryPolicy>
  on_start / on_success / on_failure: Vec<HookSpec>   // webhook hooks
  tags: BTreeMap<String,String>
  created_at / updated_at: Instant (as epoch millis)
}

TaskSpec {
  name: String          // unique within workflow
  handler: HandlerId    // which plugin to invoke
  input: JsonValue      // static or templated input
  depends_on: Vec<String>
  timeout: Option<Duration>
  retry: Option<RetryPolicy>   // overrides workflow default
}

Run {
  id: RunId             // rn_...
  workflow_id / def_snapshot: (name, version, tenant)
  input: JsonValue
  status: RunStatus     // Queued | Running | Succeeded | Failed | Cancelled | TimedOut
  attempts: u32
  next_attempt_at: EpochMillis
  started_at / finished_at / deadline_at: Option<EpochMillis>
  error: Option<RunError>
  output: Option<JsonValue>
  tags: BTreeMap<String,String>
  created_at: EpochMillis
}

TaskRun {
  id: TaskRunId         // tr_...
  run_id, task_name
  status: TaskStatus    // Pending | Running | Succeeded | Failed | Skipped
  attempts: u32
  last_error: Option<String>
  started_at / finished_at: Option<EpochMillis>
  output: Option<JsonValue>
}

QueueEntry {
  run_id, tenant, priority, scheduled_for: EpochMillis, claim_token
}

RunStatus cube:
  Queued -> Running            (claimed by worker)
  Queued -> Cancelled          (operator cancel before dispatch)
  Running -> Succeeded
  Running -> Failed            (exhausted retries)
  Running -> Cancelled
  Running -> TimedOut
  Failed  -> Queued            (retry scheduled)
  Failed  -> Cancelled
  Succeeded / Cancelled / TimedOut -> terminal
```

Run-status legality is centralised in `state::run_fsm`, task status in
`state::task_fsm`; both are pure, table-driven, and property-tested. Injecting
a "widened" transition here is a deliberate candidate for the defect
catalogue later.

## Cross-cutting decisions

- **Determinism**: all timing flows through `clock::Clock`; production uses
  `SystemClock`, tests use `ManualClock`. No test depends on wall-clock
  timing, goroutine/task scheduling order, or the network.
- **Reproducibility**: pinned toolchain (`rust-toolchain.toml`, channel
  1.96.1) and pinned third-party dependencies. The SQL backend is embedded
  (no external database server), so a fresh clone builds and tests green.
- The Postgres backend from the original plan is replaced by the embedded
  SQL backend to keep the suite fully offline and reproducible; the `Store`
  trait is backend-agnostic by design so a driver for Postgres can be layered
  on without touching domain code. Recorded as a deviation in `PLAN.md`.

## Benchmark annex

Runvane is built as source material for AI-agent benchmark tasks (see
`PLAN.md` → "Benchmark-preparation engine"). Consequences:

- The HTTP/gRPC/dashboard surfaces are designed so a meaningful subset of
  injected defects has an operator-visible, screenshottable or transcriptable
  symptom; internal-state defects fall back to captured failing-test output.
- Regression tests destined for task packaging are tagged `[F2P]`/`[P2P]`
  in Phase 8 while intent is fresh.
- The defect manifest lives at `internal-bench/defects.yaml`, by construction
  outside the production module tree (`src/`), so no task extraction ever
  ships it to a solver.
- Golden baseline semantics: the Phase-9 tag is the canonical all-fixed
  reference; every Phase-10 defect is an isolated revertible commit on top of
  it, so "broken" = golden minus its single revert.