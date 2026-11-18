//! The `serve` command: boot the control plane from a [`crate::config::Config`].
//!
//! Wiring order matters for determinism: the store is opened first (running
//! SQLite migrations), then the registry, then the shared clock; the API
//! state snapshots a boot timestamp from that same clock. The scheduler, if
//! enabled, runs on a dedicated thread so HTTP and gRPC handlers never block
//! on dispatch; shutdown is cooperative through an `AtomicBool` + graceful
//! axum shutdown, so a CI ctrl-c leaves the process in a clean state.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::api::{AppState, build_router, GrpcService};
use crate::clock::{Clock, SystemClock};
use crate::config::Config;
use crate::error::RunvaneError;
use crate::events::dispatch::LoggingSink;
use crate::events::watcher::Watcher;
use crate::plugins::handler::Registry;
use crate::scheduler::pool::{Dispatcher, WorkerPool};
use crate::scheduler::reap::reap_expired_leases;
use crate::{PRODUCT_NAME, VERSION};

/// Arguments accepted by `runvane serve`.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct ServeArgs {
    /// TOML configuration file. Later layers (env, flags) still apply.
    #[arg(long, value_name = "PATH", env = "RUNVANE_CONFIG")]
    pub config: Option<String>,

    /// Bind host for both listeners.
    #[arg(long, value_name = "HOST")]
    pub host: Option<String>,

    /// HTTP bind port.
    #[arg(long, value_name = "PORT")]
    pub http_port: Option<u16>,

    /// gRPC bind port.
    #[arg(long, value_name = "PORT")]
    pub grpc_port: Option<u16>,

    /// Worker-pool size; 0 disables the scheduler.
    #[arg(long, value_name = "N")]
    pub workers: Option<usize>,

    /// SQLite file path; default is the in-memory store.
    #[arg(long, value_name = "PATH")]
    pub sqlite: Option<String>,

    /// RUST_LOG-style tracing filter.
    #[arg(long, value_name = "FILTER")]
    pub log_filter: Option<String>,

    /// Queue lease length in milliseconds.
    #[arg(long, value_name = "MS")]
    pub lease_ms: Option<i64>,

    /// Lease-reap maintenance interval in milliseconds.
    #[arg(long, value_name = "MS")]
    pub reap_ms: Option<i64>,
}

/// Deterministic seed for scheduler RNGs so a clean boot produces
/// reproducible backoff traces (matches the fixed-seed test strategy).
const SCHEDULER_SEED: u64 = 0x5255_4E56_5EED_0001;
/// Max queue entries scanned by one dispatch round.
const DISPATCH_BATCH: usize = 128;

/// Resolves the effective configuration for a serve invocation.
pub fn resolved_config(args: &ServeArgs) -> Result<Config, RunvaneError> {
    let env: std::collections::BTreeMap<String, String> = std::env::vars().collect();
    let mut cfg = match &args.config {
        Some(path) => Config::from_file(Path::new(path))?,
        None => Config::defaults(),
    };
    apply_config_layers(&mut cfg, &env, args)
}

/// The pure layering step of [`resolved_config`], factored so the precedence
/// contract (flags > env > file > defaults) is testable without process-global
/// state.
fn apply_config_layers(
    cfg: &mut Config,
    env: &std::collections::BTreeMap<String, String>,
    args: &ServeArgs,
) -> Result<Config, RunvaneError> {
    cfg.apply_env_map(env)?;
    if let Some(v) = &args.host {
        cfg.host = v.clone();
    }
    if let Some(v) = args.http_port {
        cfg.http_port = v;
    }
    if let Some(v) = args.grpc_port {
        cfg.grpc_port = v;
    }
    if let Some(v) = args.workers {
        cfg.workers = v;
    }
    if let Some(v) = &args.log_filter {
        cfg.log_filter = v.clone();
    }
    if let Some(v) = &args.sqlite {
        cfg.sqlite_path = Some(v.clone());
    }
    if let Some(v) = args.lease_ms {
        cfg.lease_ms = v;
    }
    if let Some(v) = args.reap_ms {
        cfg.reap_interval_ms = v;
    }
    cfg.validate()?;
    Ok(cfg.clone())
}

/// Opens the backend selected by the configuration.
fn open_store(cfg: &Config) -> Result<Arc<dyn crate::persistence::Store>, RunvaneError> {
    match &cfg.sqlite_path {
        Some(path) => {
            #[cfg(feature = "sqlite")]
            {
                let store = crate::persistence::SqliteStore::open(path)
                    .map_err(RunvaneError::Storage)?;
                Ok(Arc::new(store))
            }
            #[cfg(not(feature = "sqlite"))]
            {
                Err(RunvaneError::Config(format!(
                    "sqlite_path set but the binary was built without the `sqlite` feature (sqlite_path={path:?})"
                )))
            }
        }
        None => Ok(Arc::new(crate::persistence::memory::MemoryStore::new())),
    }
}

/// Runs the control plane until SIGINT/SIGTERM or the HTTP listener drops.
pub async fn serve(args: &ServeArgs) -> Result<(), RunvaneError> {
    let cfg = resolved_config(args)?;
    tracing::debug!(?cfg, "serving with resolved configuration");

    let store = open_store(&cfg)?;
    tracing::debug!(
        cache_capacity = crate::persistence::lru_store::DEFAULT_CACHE_CAPACITY,
        "wrapping store with lru read cache"
    );
    let store: Arc<dyn crate::persistence::Store> =
        Arc::new(crate::persistence::lru_store::LruStore::wrap(store));
    let registry = Arc::new(Registry::new());
    registry.install_builtins();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let boot_ms = clock.now_ms();

    let state = Arc::new(AppState {
        store: Arc::clone(&store),
        registry: Arc::clone(&registry),
        clock: Arc::clone(&clock),
        boot_ms,
        metrics: crate::telemetry::shared(),
    });
    let router = build_router(Arc::clone(&state));
    let grpc_service = GrpcService::new(Arc::clone(&state)).into_server();

    tracing::info!(
        product = PRODUCT_NAME,
        version = VERSION,
        workers = cfg.workers,
        store = if cfg.sqlite_path.is_some() { "sqlite" } else { "memory" },
        "booted"
    );

    let stop = Arc::new(AtomicBool::new(false));
    if cfg.workers > 0 {
        spawn_scheduler_thread(SchedulerArgs {
            workers: cfg.workers,
            lease_ms: cfg.lease_ms,
            reap_ms: cfg.reap_interval_ms,
            store: Arc::clone(&store),
            registry: Arc::clone(&registry),
            clock: Arc::clone(&clock),
            metrics: Arc::clone(&state.metrics),
            stop: Arc::clone(&stop),
        })?;
    } else {
        tracing::info!("scheduler disabled (workers = 0)");
    }

    let http_listener = tokio::net::TcpListener::bind(cfg.http_addr())
        .await
        .map_err(|e| RunvaneError::Server(format!("cannot bind http listener: {e}")))?;
    tracing::info!(addr = %cfg.http_addr(), "http api listening");

    let grpc_addr = cfg.grpc_addr();
    let grpc_task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(grpc_service)
            .serve(grpc_addr.parse().unwrap())
            .await
    });
    tracing::info!(addr = %cfg.grpc_addr(), "grpc api listening");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(());
    });
    tracing::info!("control plane up; press ctrl-c to stop");

    axum::serve(http_listener, router)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .await
        .map_err(|e| RunvaneError::Server(format!("http listener failed: {e}")))?;

    tracing::info!("shutting down scheduler and grpc listener");
    stop.store(true, Ordering::SeqCst);
    grpc_task.abort();
    Ok(())
}

/// Everything the scheduler thread needs over its whole lifetime, grouped to
/// keep the spawn call site legible.
struct SchedulerArgs {
    workers: usize,
    lease_ms: i64,
    reap_ms: i64,
    store: Arc<dyn crate::persistence::Store>,
    registry: Arc<Registry>,
    clock: Arc<dyn Clock>,
    metrics: Arc<crate::telemetry::Metrics>,
    stop: Arc<AtomicBool>,
}

/// Spawns the scan/dispatch/reap loop on its own thread. The dispatcher keeps
/// a thread-pool of worker threads active for the server's whole lifetime; the
/// `stop` flag yields cooperatively between rounds so shutdown is prompt.
fn spawn_scheduler_thread(args: SchedulerArgs) -> Result<(), RunvaneError> {
    let SchedulerArgs {
        workers,
        lease_ms,
        reap_ms,
        store,
        registry,
        clock,
        metrics,
        stop,
    } = args;
    let pool = WorkerPool::spawn(workers, registry, Arc::clone(&store), Arc::clone(&clock), SCHEDULER_SEED);
    let poll = Duration::from_millis(reap_ms.max(1) as u64);
    // Webhook watch: log deliveries by default; operators can point hooks at
    // loopback receivers or swap in a stronger sink. Anchored at boot so a
    // clean start replays nothing.
    let mut watcher = Watcher::with_sink(Arc::new(LoggingSink), clock.as_ref());
    std::thread::spawn(move || {
        // Build the dispatcher inside the thread so its borrowed store/clock
        // references live exactly as long as the owning `Arc`s do.
        let dispatcher = Dispatcher::new(
            store.as_ref(),
            clock.as_ref(),
            pool,
            DISPATCH_BATCH,
            lease_ms,
        );
        while !stop.load(Ordering::SeqCst) {
            let stats = dispatcher.step();
            metrics
                .dispatches_total
                .fetch_add(1, Ordering::Relaxed);
            let reap = reap_expired_leases(store.as_ref(), clock.as_ref());
            let watch = watcher.poll(store.as_ref(), clock.now_ms());
            metrics
                .watch_terminals_total
                .fetch_add(watch.finished as u64, Ordering::Relaxed);
            metrics
                .event_deliveries_total
                .fetch_add(watch.dispatched as u64, Ordering::Relaxed);
            metrics
                .event_deliveries_ok_total
                .fetch_add(watch.dispatched as u64, Ordering::Relaxed);
            tracing::debug!(
                scanned = stats.scanned,
                claimed = stats.claimed,
                reap_stats = ?reap,
                events = ?watch,
            );
            std::thread::sleep(poll);
        }
        tracing::debug!("scheduler thread stopped");
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env() -> std::collections::BTreeMap<String, String> {
        std::collections::BTreeMap::new()
    }

    #[test]
    fn flags_win_over_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("runvane.toml");
        std::fs::write(&file, "http_port = 7001\nworkers = 1\n").unwrap();
        let mut args = ServeArgs {
            config: Some(file.to_string_lossy().into_owned()),
            http_port: Some(7002),
            ..ServeArgs::default()
        };
        let mut cfg = Config::from_file(&file).unwrap();
        let cfg2 = apply_config_layers(&mut cfg, &no_env(), &args).unwrap();
        assert_eq!(cfg2.http_port, 7002, "flag must beat file");
        assert_eq!(cfg2.workers, 1, "file value lands when no flag overrides");

        args.http_port = None;
        let mut fresh = Config::from_file(&file).unwrap();
        let cfg3 = apply_config_layers(&mut fresh, &no_env(), &args).unwrap();
        assert_eq!(cfg3.http_port, 7001, "file value restored once flag removed");
    }

    #[test]
    fn flags_win_over_env() {
        let mut env = no_env();
        env.insert("RUNVANE_HTTP_PORT".to_owned(), "8001".to_owned());
        env.insert("RUNVANE_WORKERS".to_owned(), "2".to_owned());
        let args = ServeArgs {
            http_port: Some(8002),
            ..ServeArgs::default()
        };
        let mut cfg = Config::defaults();
        let out = apply_config_layers(&mut cfg, &env, &args).unwrap();
        assert_eq!(out.http_port, 8002, "flag must beat env");
        assert_eq!(out.workers, 2, "env lands when no flag overrides");
    }

    #[test]
    fn env_wins_over_file_over_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("runvane.toml");
        std::fs::write(&file, "http_port = 7001\nworkers = 1\n").unwrap();
        let mut env = no_env();
        env.insert("RUNVANE_HTTP_PORT".to_owned(), "8077".to_owned());
        let args = ServeArgs {
            config: Some(file.to_string_lossy().into_owned()),
            ..ServeArgs::default()
        };
        let mut cfg = Config::from_file(&file).unwrap();
        let out = apply_config_layers(&mut cfg, &env, &args).unwrap();
        assert_eq!(out.http_port, 8077, "env must beat file");
        assert_eq!(out.workers, 1);
        assert_eq!(out.grpc_port, crate::config::DEFAULT_GRPC_PORT);
    }

    #[test]
    fn no_layers_stays_defaults() {
        let args = ServeArgs::default();
        let mut cfg = Config::defaults();
        let out = apply_config_layers(&mut cfg, &no_env(), &args).unwrap();
        assert_eq!(out, Config::defaults());
    }
}