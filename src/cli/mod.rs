//! The `runvane` command line.
//!
//! Subcommands split into operational (`serve`) and control-plane
//! (`workflows`, `runs`). The control-plane subcommands are thin shells over
//! the gRPC client in [`self::client`], so local and remote operation go over
//! the same protocol and the CLI never needs its own copy of the wire
//! contract. Output is one JSON document per invocation (pretty-printed) to
//! keep the CLI scriptable and diff-stable.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::error::RunvaneError;
use crate::{PRODUCT_TAGLINE, VERSION};

pub mod client;
pub mod completion;
pub mod serve;

/// Program-level arguments shared by every invocation.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "runvane",
    version = VERSION,
    about = PRODUCT_TAGLINE,
    long_about = "Runvane — durable distributed workflow orchestration. Operate the control plane (serve) or query/send work (workflows, runs).",
    arg_required_else_help = true,
    subcommand_required = true
)]
pub struct Cli {
    /// gRPC endpoint for control-plane subcommands. Must be a valid URI.
    #[arg(
        long,
        global = true,
        value_name = "URI",
        env = "RUNVANE_ENDPOINT",
        default_value = client::DEFAULT_ENDPOINT
    )]
    pub endpoint: String,

    #[command(subcommand)]
    /// The subcommand to execute.
    pub command: Command,
}

/// Everything `runvane` can do.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Start the HTTP + gRPC control plane.
    Serve(serve::ServeArgs),
    /// Inspect and mutate workflow definitions.
    Workflows(WorkflowCommand),
    /// Inspect and mutate runs.
    Runs(RunCommand),
    /// Statically simulate and analyze a workflow DAG without running it.
    DryRun(DryRunArgs),
    /// Display real-time control plane health and execution telemetry.
    Stats(StatsArgs),
    /// Generate shell autocompletion script for bash, zsh, or fish.
    Completion(CompletionArgs),
    /// Print product/build info.
    Version,
}

/// `runvane workflows` operations.
#[derive(Debug, Clone, Args)]
pub struct WorkflowCommand {
    #[command(subcommand)]
    /// The workflow operation to run.
    pub op: WorkflowOp,
}

/// `runvane workflows <op>` selection.
#[derive(Debug, Clone, Subcommand)]
pub enum WorkflowOp {
    /// Create (or fail on a collision) a workflow definition from a JSON file.
    Create(CreateArgs),
    /// List definitions, optionally scoped by tenant.
    List(ListArgs),
    /// Fetch one definition by tenant and name.
    Get(GetArgs),
}

/// Shared positional identifiers used by workflow subcommands.
#[derive(Debug, Clone, Args)]
pub struct CreateArgs {
    /// Tenant namespace.
    #[arg(long, value_name = "TENANT")]
    pub tenant: String,
    /// Definition name.
    #[arg(long, value_name = "NAME")]
    pub name: String,
    /// JSON file with the definition payload (tasks, retry, hooks, ...).
    #[arg(value_name = "SPEC.json")]
    pub file: PathBuf,
}

/// `runvane workflows list` flags.
#[derive(Debug, Clone, Args)]
pub struct ListArgs {
    /// Only definitions owned by this tenant.
    #[arg(short = 't', long, value_name = "TENANT")]
    pub tenant: Option<String>,
}

/// `runvane workflows get` positional pair; rejects a missing one.
#[derive(Debug, Clone, Args)]
pub struct GetArgs {
    /// Tenant namespace.
    #[arg(value_name = "TENANT")]
    pub tenant: String,
    /// Definition name.
    #[arg(value_name = "NAME")]
    pub name: String,
}

/// `runvane runs` operations.
#[derive(Debug, Clone, Args)]
pub struct RunCommand {
    #[command(subcommand)]
    /// The run operation to run.
    pub op: RunOp,
}

/// `runvane runs <op>` selection.
#[derive(Debug, Clone, Subcommand)]
pub enum RunOp {
    /// Submit a new run.
    Submit(SubmitArgs),
    /// List runs, optionally filtered.
    List(RunListArgs),
    /// Fetch one run by id.
    Get(GetRunArgs),
    /// Cancel a run (Queued or Running only).
    Cancel(CancelArgs),
}

/// `runvane runs submit` flags.
#[derive(Debug, Clone, Args)]
pub struct SubmitArgs {
    /// Tenant namespace.
    #[arg(long, value_name = "TENANT")]
    pub tenant: String,
    /// Definition name.
    #[arg(long, value_name = "NAME")]
    pub name: String,
    /// JSON file with the run input; defaults to `{}`.
    #[arg(long, value_name = "INPUT.json")]
    pub input: Option<PathBuf>,
    /// Key=value tag to stamp on the run; repeatable.
    #[arg(short = 't', long, value_name = "KEY=VALUE")]
    pub tag: Vec<String>,
}

/// `runvane runs list` flags.
#[derive(Debug, Clone, Args)]
pub struct RunListArgs {
    /// Only runs of this tenant.
    #[arg(long, value_name = "TENANT")]
    pub tenant: Option<String>,
    /// Only runs of this definition name.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    /// Only runs with this status.
    #[arg(long, value_name = "STATUS")]
    pub status: Option<String>,
    /// Max results (server caps this).
    #[arg(long, default_value_t = 100, value_name = "N")]
    pub limit: u32,
}

/// `runvane runs get` flags.
#[derive(Debug, Clone, Args)]
pub struct GetRunArgs {
    /// Run id (`rn_...`).
    #[arg(long, value_name = "RUN_ID")]
    pub id: String,
}

/// `runvane runs cancel` flags.
#[derive(Debug, Clone, Args)]
pub struct CancelArgs {
    /// Run id (`rn_...`).
    #[arg(long, value_name = "RUN_ID")]
    pub id: String,
}

/// `runvane dry-run` arguments.
#[derive(Debug, Clone, Args)]
pub struct DryRunArgs {
    /// Path to the workflow JSON definition file.
    #[arg(value_name = "SPEC.json")]
    pub file: PathBuf,
    /// Output format (`text` or `json`).
    #[arg(short = 'f', long, default_value = "text")]
    pub format: String,
}

/// `runvane stats` arguments.
#[derive(Debug, Clone, Args)]
pub struct StatsArgs {
    /// Output format (`text` or `json`).
    #[arg(short = 'f', long, default_value = "text")]
    pub format: String,
}

/// `runvane completion` arguments.
#[derive(Debug, Clone, Args)]
pub struct CompletionArgs {
    /// Target shell (`bash`, `zsh`, or `fish`).
    #[arg(value_name = "SHELL")]
    pub shell: String,
}

impl Cli {
    /// Parses argv without exiting (testable) — the clap top-level entry for
    /// `main`.
    pub fn try_parse_from<I, T>(itr: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        <Self as Parser>::try_parse_from(itr)
    }
}

/// Runs `workflows` and `runs` subcommands against the given endpoint,
/// printing the outcome to stdout. Kept separate from the async dispatch so
/// the happy path is unit-testable without a live server.
pub async fn execute(cli: &Cli) -> Result<(), RunvaneError> {
    match &cli.command {
        Command::Serve(args) => serve::serve(args).await,
        Command::Workflows(wf) => workflows(cli, wf).await,
        Command::Runs(runs) => run_commands(cli, runs).await,
        Command::DryRun(args) => dry_run(args),
        Command::Stats(args) => stats_command(cli, args).await,
        Command::Completion(args) => completion_command(args),
        Command::Version => {
            println!("{} {VERSION} — {PRODUCT_TAGLINE}", crate::PRODUCT_NAME);
            Ok(())
        }
    }
}

async fn workflows(cli: &Cli, wf: &WorkflowCommand) -> Result<(), RunvaneError> {
    let mut client = client::Client::connect(&cli.endpoint).await?;
    match &wf.op {
        WorkflowOp::Create(args) => {
            let raw = std::fs::read_to_string(&args.file).map_err(|e| {
                RunvaneError::Config(format!("cannot read {}: {e}", args.file.display()))
            })?;
            let spec = build_workflow_spec(&args.tenant, &args.name, &raw)?;
            let resp = client.create_workflow(spec).await?;
            println!("{}", client::render_json(&resp.definition_json)?);
        }
        WorkflowOp::List(args) => {
            let resp = client
                .list_workflows(args.tenant.as_deref().unwrap_or(""))
                .await?;
            println!("{}", client::render_json_list(&resp.definition_json)?);
        }
        WorkflowOp::Get(args) => {
            let resp = client.get_workflow(&args.tenant, &args.name).await?;
            println!("{}", client::render_json(&resp.definition_json)?);
        }
    }
    Ok(())
}

async fn run_commands(cli: &Cli, runs: &RunCommand) -> Result<(), RunvaneError> {
    let mut client = client::Client::connect(&cli.endpoint).await?;
    match &runs.op {
        RunOp::Submit(args) => {
            let input_json = match &args.input {
                Some(path) => {
                    let raw = std::fs::read_to_string(path).map_err(|e| {
                        RunvaneError::Config(format!("cannot read {}: {e}", path.display()))
                    })?;
                    client::parse_json_bytes(&raw, "input")?
                }
                None => b"{}".to_vec(),
            };
            let tags = client::tags_to_json(&args.tag)?;
            let resp = client
                .submit_run(&args.tenant, &args.name, input_json, tags)
                .await?;
            println!("{}", client::render_json(&resp.run_json)?);
        }
        RunOp::List(args) => {
            let resp = client
                .list_runs(
                    args.tenant.as_deref().unwrap_or(""),
                    args.name.as_deref().unwrap_or(""),
                    args.status.as_deref().unwrap_or(""),
                    args.limit,
                )
                .await?;
            println!("{}", client::render_json_list(&resp.run_json)?);
        }
        RunOp::Get(args) => {
            let resp = client.get_run(&args.id).await?;
            println!("{}", client::render_json(&resp.run_json)?);
        }
        RunOp::Cancel(args) => {
            let resp = client.cancel_run(&args.id).await?;
            println!("{}", client::render_json(&resp.run_json)?);
        }
    }
    Ok(())
}

/// The retry subsection accepted by spec files (mirrors `RetryPolicy` fields).
#[derive(Debug, serde::Deserialize)]
struct RetryFile {
    max_attempts: Option<u32>,
    base_delay_ms: Option<i64>,
    max_delay_ms: Option<i64>,
    multiplier: Option<f64>,
    backoff: Option<String>,
    jitter: Option<String>,
    retryable_only: Option<bool>,
}

/// The lifecycle-hook block accepted by spec files.
#[derive(Debug, serde::Deserialize)]
struct HooksFile {
    on_start: Option<Vec<HookFile>>,
    on_success: Option<Vec<HookFile>>,
    on_failure: Option<Vec<HookFile>>,
    on_cancel: Option<Vec<HookFile>>,
}

/// A single hook binding in a spec file.
#[derive(Debug, serde::Deserialize)]
struct HookFile {
    webhook_url: Option<String>,
    event_filter: Option<String>,
    headers: Option<std::collections::BTreeMap<String, String>>,
}

impl RetryFile {
    fn into_proto(self) -> client::wire::RetryPolicy {
        client::wire::RetryPolicy {
            max_attempts: self.max_attempts.unwrap_or(0),
            base_delay_ms: self.base_delay_ms.unwrap_or(0) as u64,
            max_delay_ms: self.max_delay_ms.unwrap_or(0) as u64,
            multiplier: self.multiplier.unwrap_or(1.0),
            backoff: self.backoff.unwrap_or_default(),
            jitter: self.jitter.unwrap_or_default(),
            retryable_only: self.retryable_only.unwrap_or(false),
        }
    }
}

fn dry_run(args: &DryRunArgs) -> Result<(), RunvaneError> {
    let raw = std::fs::read_to_string(&args.file)
        .map_err(|e| RunvaneError::Config(format!("cannot read {}: {e}", args.file.display())))?;

    let def: crate::domain::workflow::WorkflowDef = match serde_json::from_str(&raw) {
        Ok(d) => d,
        Err(_) => {
            let spec = build_workflow_spec("preview", "dryrun", &raw)?;
            let domain_spec = crate::api::payloads::WorkflowSpec {
                tenant: spec.tenant,
                name: spec.name,
                description: if spec.description.is_empty() {
                    None
                } else {
                    Some(spec.description)
                },
                tasks: spec
                    .tasks
                    .into_iter()
                    .map(|t| {
                        let input: serde_json::Value = if t.input_json.is_empty() {
                            serde_json::json!({})
                        } else {
                            serde_json::from_slice(&t.input_json).unwrap_or(serde_json::json!({}))
                        };
                        crate::api::payloads::TaskSpecPayload {
                            name: t.name,
                            handler: t.handler,
                            input: Some(input),
                            depends_on: Some(t.depends_on),
                            timeout_ms: if t.timeout_ms == 0 {
                                None
                            } else {
                                Some(t.timeout_ms)
                            },
                            retry: None,
                            meta: None,
                        }
                    })
                    .collect(),
                timeout_ms: if spec.timeout_ms == 0 {
                    None
                } else {
                    Some(spec.timeout_ms)
                },
                retry: None,
                default_priority: None,
                hooks: None,
                tags: None,
            };
            domain_spec.into_definition(0)?
        }
    };

    let report = crate::engine::dry_run::simulate_workflow(&def)
        .map_err(|e| RunvaneError::Server(format!("simulation failed: {e}")))?;

    if args.format == "json" {
        let json = serde_json::to_string_pretty(&report)
            .map_err(|e| RunvaneError::Server(e.to_string()))?;
        println!("{json}");
    } else {
        print!("{}", report.format_text());
    }
    Ok(())
}

async fn stats_command(cli: &Cli, args: &StatsArgs) -> Result<(), RunvaneError> {
    let mut client = client::Client::connect(&cli.endpoint).await?;
    let health = client.health().await?;
    let workflows = client.list_workflows("").await?;
    let runs = client.list_runs("", "", "", 500).await?;

    let now_ms = health.now_ms;
    let uptime_s = (now_ms - health.booted_at_ms).max(0) / 1000;

    let mut succeeded = 0usize;
    let mut running = 0usize;
    let mut queued = 0usize;
    let mut failed = 0usize;

    for r_bytes in &runs.run_json {
        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(r_bytes) {
            if let Some(status) = val.get("status").and_then(|s| s.as_str()) {
                match status {
                    "succeeded" => succeeded += 1,
                    "running" => running += 1,
                    "queued" => queued += 1,
                    _ => failed += 1,
                }
            }
        }
    }

    if args.format == "json" {
        let payload = serde_json::json!({
            "status": health.status,
            "uptime_seconds": uptime_s,
            "queue_depth": health.queue_depth,
            "workflow_count": workflows.definition_json.len(),
            "runs": {
                "total": runs.run_json.len(),
                "succeeded": succeeded,
                "running": running,
                "queued": queued,
                "failed": failed,
            }
        });
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
    } else {
        println!("=== Runvane Control Plane Diagnostics ===");
        println!("Status:       {}", health.status);
        println!("Uptime:       {}s", uptime_s);
        println!("Queue Depth:  {}", health.queue_depth);
        println!("Workflows:    {}", workflows.definition_json.len());
        println!("Total Runs:   {}", runs.run_json.len());
        println!("  - Succeeded: {}", succeeded);
        println!("  - Running:   {}", running);
        println!("  - Queued:    {}", queued);
        println!("  - Failed:    {}", failed);
    }
    Ok(())
}

fn completion_command(args: &CompletionArgs) -> Result<(), RunvaneError> {
    match completion::Shell::parse(&args.shell) {
        Some(sh) => {
            print!("{}", completion::generate_completion(sh));
            Ok(())
        }
        None => Err(RunvaneError::Config(format!(
            "unsupported shell '{}'; expected 'bash', 'zsh', or 'fish'",
            args.shell
        ))),
    }
}

/// Turns a JSON-file definition payload into the proto spec, sending free-form
/// documents as JSON bytes. The file shape mirrors the common payload field
/// names so the CLI and HTTP/gRPC clients accept the same artifact.
fn build_workflow_spec(
    tenant: &str,
    name: &str,
    raw: &str,
) -> Result<client::wire::WorkflowSpec, RunvaneError> {
    #[derive(serde::Deserialize)]
    struct SpecFile {
        description: Option<String>,
        default_retry: Option<RetryFile>,
        default_timeout_ms: Option<i64>,
        default_priority: Option<String>,
        tags: Option<std::collections::BTreeMap<String, String>>,
        hooks: Option<HooksFile>,
        tasks: Vec<TaskFile>,
    }
    #[derive(serde::Deserialize)]
    struct TaskFile {
        name: String,
        handler: String,
        #[serde(default)]
        depends_on: Vec<String>,
        input: Option<serde_json::Value>,
        timeout_ms: Option<i64>,
        retry: Option<RetryFile>,
        meta: Option<serde_json::Value>,
    }

    let spec: SpecFile = serde_json::from_str(raw)
        .map_err(|e| RunvaneError::Config(format!("spec file does not parse: {e}")))?;

    let mut tasks = Vec::with_capacity(spec.tasks.len());
    for task in spec.tasks {
        let input_json = match task.input {
            None | Some(serde_json::Value::Null) => Vec::new(),
            Some(json) => {
                serde_json::to_vec(&json).map_err(|e| RunvaneError::Server(e.to_string()))?
            }
        };
        let meta_json = match task.meta {
            None | Some(serde_json::Value::Null) => Vec::new(),
            Some(json) => {
                serde_json::to_vec(&json).map_err(|e| RunvaneError::Server(e.to_string()))?
            }
        };
        tasks.push(client::wire::TaskSpec {
            name: task.name,
            handler: task.handler,
            depends_on: task.depends_on,
            input_json,
            timeout_ms: task.timeout_ms.unwrap_or(0) as u64,
            retry: task.retry.map(RetryFile::into_proto),
            meta_json,
        });
    }
    let tags_json = match spec.tags {
        None => Vec::new(),
        Some(map) => serde_json::to_vec(&map).map_err(|e| RunvaneError::Server(e.to_string()))?,
    };
    let hooks = hooks_file_into_proto(spec.hooks.as_ref());
    Ok(client::wire::WorkflowSpec {
        tenant: tenant.to_owned(),
        name: name.to_owned(),
        description: spec.description.unwrap_or_default(),
        retry: spec.default_retry.map(RetryFile::into_proto),
        timeout_ms: spec.default_timeout_ms.unwrap_or(0) as u64,
        default_priority: spec.default_priority.unwrap_or_default(),
        tags_json,
        tasks,
        hooks,
    })
}

fn hook_spec_into_proto(hook: &HookFile) -> client::wire::HookSpec {
    client::wire::HookSpec {
        webhook_url: hook.webhook_url.clone().unwrap_or_default(),
        event_filter: hook.event_filter.clone().unwrap_or_default(),
        headers: hook
            .headers
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect(),
    }
}

fn hooks_file_into_proto(hooks: Option<&HooksFile>) -> Option<client::wire::HooksSpec> {
    let hooks = hooks?;
    let empty = || Vec::with_capacity(0);
    Some(client::wire::HooksSpec {
        on_start: hooks
            .on_start
            .as_ref()
            .map(|hs| hs.iter().map(hook_spec_into_proto).collect())
            .unwrap_or_else(empty),
        on_success: hooks
            .on_success
            .as_ref()
            .map(|hs| hs.iter().map(hook_spec_into_proto).collect())
            .unwrap_or_else(empty),
        on_failure: hooks
            .on_failure
            .as_ref()
            .map(|hs| hs.iter().map(hook_spec_into_proto).collect())
            .unwrap_or_else(empty),
        on_cancel: hooks
            .on_cancel
            .as_ref()
            .map(|hs| hs.iter().map(hook_spec_into_proto).collect())
            .unwrap_or_else(empty),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut argv = vec!["runvane"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv)
    }

    #[test]
    fn version_command_parses() {
        let cli = parse(&["version"]).unwrap();
        assert!(matches!(cli.command, Command::Version));
    }

    #[test]
    fn serve_flags_parse() {
        let cli = parse(&[
            "--endpoint",
            "http://127.0.0.1:1234",
            "serve",
            "--http-port",
            "8123",
            "--workers",
            "0",
            "--sqlite",
            "runvane.db",
        ])
        .unwrap();
        assert_eq!(cli.endpoint, "http://127.0.0.1:1234");
        let Command::Serve(args) = &cli.command else {
            panic!("expected serve, got {:?}", cli.command);
        };
        assert_eq!(args.http_port, Some(8123));
        assert_eq!(args.workers, Some(0));
        assert_eq!(args.sqlite.as_deref(), Some("runvane.db"));
    }

    #[test]
    fn endpoint_env_falls_back_to_default() {
        // No endpoint flag: the clap `env` + default_value machinery applies.
        let cli = parse(&["version"]).unwrap();
        assert_eq!(cli.endpoint, client::DEFAULT_ENDPOINT);
    }

    #[test]
    fn missing_subcommand_is_rejected() {
        assert!(parse(&["--endpoint", "x"]).is_err());
    }

    #[test]
    fn workflows_get_requires_tenant_and_name() {
        let cli = parse(&["workflows", "get", "acme", "ship"]).unwrap();
        let Command::Workflows(wf) = &cli.command else {
            panic!("expected workflows, got {:?}", cli.command);
        };
        let WorkflowOp::Get(args) = &wf.op else {
            panic!("expected get, got {:?}", wf.op);
        };
        assert_eq!(args.tenant, "acme");
        assert_eq!(args.name, "ship");
    }

    #[test]
    fn runs_submit_accepts_tags() {
        let cli = parse(&[
            "runs", "submit", "--tenant", "a", "--name", "b", "-t", "k=v", "-t", "x=y",
        ])
        .unwrap();
        let Command::Runs(runs) = &cli.command else {
            panic!("expected runs, got {:?}", cli.command);
        };
        let RunOp::Submit(args) = &runs.op else {
            panic!("expected submit, got {:?}", runs.op);
        };
        assert_eq!(args.tag, vec!["k=v", "x=y"]);
    }

    #[test]
    fn spec_file_builds_the_proto_payload() {
        let raw = r#"{
            "description": "ship it",
            "default_retry": {"max_attempts": 3, "base_delay_ms": 100, "backoff": "exponential"},
            "default_timeout_ms": 30000,
            "default_priority": "high",
            "tags": {"team": "infra"},
            "hooks": {"on_success": [{"webhook_url": "http://hooks:8080/ship", "event_filter": "run.succeeded", "headers": {"X-Team": "infra"}}]},
            "tasks": [
                {"name": "build", "handler": "runvane.echo", "input": {"step": "build"}},
                {"name": "deploy", "handler": "runvane.echo", "depends_on": ["build"], "meta": {"audit": true}}
            ]
        }"#;
        let spec = build_workflow_spec("acme", "ship", raw).unwrap();
        assert_eq!(spec.tenant, "acme");
        assert_eq!(spec.name, "ship");
        assert_eq!(spec.description, "ship it");
        assert_eq!(spec.timeout_ms, 30_000);
        assert_eq!(spec.default_priority, "high");
        assert_eq!(spec.tasks.len(), 2);
        assert_eq!(spec.tasks[0].handler, "runvane.echo");
        assert_eq!(spec.tasks[1].depends_on, vec!["build"]);
        assert!(!spec.tasks[0].input_json.is_empty());
        let retry = spec.retry.unwrap();
        assert_eq!(retry.max_attempts, 3);
        assert_eq!(retry.backoff, "exponential");
        let hooks = spec.hooks.unwrap();
        assert_eq!(hooks.on_success.len(), 1);
        assert_eq!(hooks.on_success[0].webhook_url, "http://hooks:8080/ship");
        assert_eq!(hooks.on_success[0].event_filter, "run.succeeded");
    }

    #[test]
    fn spec_file_rejects_unknown_fields() {
        // serde default: unknown top-level keys are allowed, but the retry
        // subsection is strict — a typo there must not come through.
        let raw = r#"{"tasks":[{"name":"a","handler":"h","retry":{"max_attempts":"two"}}]}"#;
        let err = build_workflow_spec("t", "n", raw).unwrap_err();
        assert!(matches!(err, RunvaneError::Config(_)));
    }

    #[test]
    fn spec_file_requires_a_task_list() {
        let err = build_workflow_spec("t", "n", r#"{"description":"x"}"#).unwrap_err();
        assert!(matches!(err, RunvaneError::Config(_)));
    }

    /// Boots an in-process gRPC control plane on an ephemeral port.
    async fn spawn_test_server() -> (String, tokio::task::JoinHandle<()>) {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let socket: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let state = std::sync::Arc::new(crate::api::AppState {
            store: std::sync::Arc::new(crate::persistence::memory::MemoryStore::new()),
            registry: std::sync::Arc::new(crate::plugins::handler::Registry::new()),
            clock: std::sync::Arc::new(crate::clock::ManualClock::at(1_720_000_000_000)),
            boot_ms: 1_720_000_000_000,
            metrics: crate::telemetry::shared(),
            auth: crate::auth::AuthConfig::default(),
            audit: std::sync::Arc::new(crate::audit::memory::MemoryAuditLogger::default()),
        });
        let svc = crate::api::GrpcService::new(state).into_server();
        let handle = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(svc)
                .serve(socket)
                .await
                .unwrap();
        });
        (format!("http://127.0.0.1:{port}"), handle)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn end_to_end_create_submit_cancel_round_trip() {
        let (endpoint, handle) = spawn_test_server().await;
        let dir = tempfile::tempdir().unwrap();
        let spec_file = dir.path().join("ship.json");
        std::fs::write(
            &spec_file,
            r#"{"description":"ci","tasks":[{"name":"build","handler":"runvane.echo","input":{"step":"build"}}]}"#,
        )
        .unwrap();

        // Create via the workflows create command.
        let cli = parse(&[
            "--endpoint",
            &endpoint,
            "workflows",
            "create",
            "--tenant",
            "acme",
            "--name",
            "ship",
            spec_file.to_str().unwrap(),
        ])
        .unwrap();
        execute(&cli).await.unwrap();

        // Submit a run with a tag file's worth of metadata.
        let cli = parse(&[
            "--endpoint",
            &endpoint,
            "runs",
            "submit",
            "--tenant",
            "acme",
            "--name",
            "ship",
            "-t",
            "env=prod",
        ])
        .unwrap();
        execute(&cli).await.unwrap();

        // Fetch and cancel it.
        let cli = parse(&[
            "--endpoint",
            &endpoint,
            "runs",
            "list",
            "--tenant",
            "acme",
            "--limit",
            "10",
        ])
        .unwrap();
        execute(&cli).await.unwrap();
        handle.abort();
    }

    #[tokio::test]
    async fn dry_run_cli_simulation() {
        let dir = tempfile::tempdir().unwrap();
        let spec_file = dir.path().join("pipeline.json");
        std::fs::write(
            &spec_file,
            serde_json::json!({
                "tasks": [
                    {"name": "fetch", "handler": "runvane.echo"},
                    {"name": "process", "handler": "runvane.echo", "depends_on": ["fetch"]}
                ]
            })
            .to_string(),
        )
        .unwrap();

        let cli = parse(&["dry-run", spec_file.to_str().unwrap()]).unwrap();
        execute(&cli).await.unwrap();

        let cli_json = parse(&["dry-run", "-f", "json", spec_file.to_str().unwrap()]).unwrap();
        execute(&cli_json).await.unwrap();
    }

    #[tokio::test]
    async fn stats_cli_diagnostics() {
        let (endpoint, handle) = spawn_test_server().await;

        let cli = parse(&["--endpoint", &endpoint, "stats"]).unwrap();
        execute(&cli).await.unwrap();

        let cli_json = parse(&["--endpoint", &endpoint, "stats", "-f", "json"]).unwrap();
        execute(&cli_json).await.unwrap();

        handle.abort();
    }

    #[tokio::test]
    async fn completion_cli_generation() {
        for shell in ["bash", "zsh", "fish"] {
            let cli = parse(&["completion", shell]).unwrap();
            execute(&cli).await.unwrap();
        }
        let bad = parse(&["completion", "unsupported"]).unwrap();
        assert!(execute(&bad).await.is_err());
    }
}
