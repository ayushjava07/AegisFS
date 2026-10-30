//! Server configuration: defaults, a TOML file layer, and environment
//! overrides.
//!
//! Precedence (lowest to highest): built-in defaults, the TOML file passed
//! via `--config` or `RUNVANE_CONFIG`, `RUNVANE_*` environment variables.
//! The CLI flags set by `serve` win last but live in [`crate::cli`]; this
//! module owns the value shape only.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::error::RunvaneError;

/// Default HTTP listen port.
pub const DEFAULT_HTTP_PORT: u16 = 8080;
/// Default gRPC listen port.
pub const DEFAULT_GRPC_PORT: u16 = 9090;
/// Default worker-pool size when the scheduler is enabled.
pub const DEFAULT_WORKERS: usize = 4;
/// Default lease length handed to claiming workers, in milliseconds.
pub const DEFAULT_LEASE_MS: i64 = 60_000;
/// Default maintenance interval for reaping expired leases, in milliseconds.
pub const DEFAULT_REAP_MS: i64 = 5_000;
/// Default max runs returned by API list queries.
pub const DEFAULT_LIST_LIMIT: usize = 100;

/// Fully-resolved server configuration.
///
/// TOML keys mirror these fields (`snake_case`); environment overrides use the
/// `RUNVANE_` prefix and the uppercase field name (`RUNVANE_HTTP_PORT`).
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Bind host for both listeners.
    pub host: String,
    /// HTTP (axum) bind port.
    pub http_port: u16,
    /// gRPC (tonic) bind port.
    pub grpc_port: u16,
    /// Worker-pool size. `0` disables the scheduler entirely.
    pub workers: usize,
    /// SQLite file path; `None` runs on the in-memory store.
    pub sqlite_path: Option<String>,
    /// Tracing filter (`RUST_LOG`-style).
    pub log_filter: String,
    /// Optional bearer token enforced on `/v1` routes.
    pub auth_token: Option<String>,
    /// Queue lease length for claiming workers.
    pub lease_ms: i64,
    /// Interval between lease-reap maintenance passes.
    pub reap_interval_ms: i64,
    /// Server default page size for run queries.
    pub default_list_limit: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            http_port: DEFAULT_HTTP_PORT,
            grpc_port: DEFAULT_GRPC_PORT,
            workers: DEFAULT_WORKERS,
            sqlite_path: None,
            log_filter: "info".to_owned(),
            auth_token: None,
            lease_ms: DEFAULT_LEASE_MS,
            reap_interval_ms: DEFAULT_REAP_MS,
            default_list_limit: DEFAULT_LIST_LIMIT,
        }
    }
}

/// The TOML document accepted by `--config`. Every field is optional; missing
/// fields keep the running defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ConfigFile {
    host: Option<String>,
    http_port: Option<u16>,
    grpc_port: Option<u16>,
    workers: Option<usize>,
    sqlite_path: Option<String>,
    log_filter: Option<String>,
    auth_token: Option<String>,
    lease_ms: Option<i64>,
    reap_interval_ms: Option<i64>,
    default_list_limit: Option<usize>,
    /// Unrecognized keys are collected here and turned into errors so a typo
    /// fails loudly instead of silently serving defaults.
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

fn unknown_key_error(keys: &BTreeMap<String, toml::Value>) -> Option<RunvaneError> {
    if keys.is_empty() {
        return None;
    }
    let mut names: Vec<&str> = keys.keys().map(String::as_str).collect();
    names.sort();
    Some(RunvaneError::Config(format!(
        "unknown config key(s): {}",
        names.join(", ")
    )))
}

impl Config {
    /// The configuration with everything at its default value.
    pub fn defaults() -> Self {
        Self::default()
    }

    /// Parses a TOML document on top of the defaults.
    pub fn from_toml_str(toml: &str) -> Result<Self, RunvaneError> {
        let file: ConfigFile = toml::from_str(toml).map_err(|e| {
            RunvaneError::Config(format!("config file does not parse: {e}"))
        })?;
        if let Some(err) = unknown_key_error(&file.unknown) {
            return Err(err);
        }
        let mut cfg = Self::default();
        if let Some(v) = file.host {
            cfg.host = v;
        }
        if let Some(v) = file.http_port {
            cfg.http_port = v;
        }
        if let Some(v) = file.grpc_port {
            cfg.grpc_port = v;
        }
        if let Some(v) = file.workers {
            cfg.workers = v;
        }
        if let Some(v) = file.sqlite_path {
            cfg.sqlite_path = Some(v);
        }
        if let Some(v) = file.log_filter {
            cfg.log_filter = v;
        }
        if let Some(v) = file.auth_token {
            cfg.auth_token = Some(v);
        }
        if let Some(v) = file.lease_ms {
            cfg.lease_ms = v;
        }
        if let Some(v) = file.reap_interval_ms {
            cfg.reap_interval_ms = v;
        }
        if let Some(v) = file.default_list_limit {
            cfg.default_list_limit = v;
        }
        cfg.validate()?;
        Ok(cfg)
    }

    /// Loads a TOML file on top of the defaults.
    pub fn from_file(path: &Path) -> Result<Self, RunvaneError> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            RunvaneError::Config(format!("cannot read config file {}: {e}", path.display()))
        })?;
        Self::from_toml_str(&text)
    }

    /// Overlays `RUNVANE_*` environment variables over the current values.
    ///
    /// Reading the process environment is wrapped so the pure overlay logic is
    /// unit-testable without mutating process-global state.
    pub fn apply_env(&mut self) -> Result<(), RunvaneError> {
        let env: BTreeMap<String, String> = std::env::vars().collect();
        self.apply_env_map(&env)
    }

    /// The pure variant of [`Config::apply_env`]; keys must be `RUNVANE_`-prefixed.
    pub fn apply_env_map(&mut self, env: &BTreeMap<String, String>) -> Result<(), RunvaneError> {
        let mut applied = BTreeMap::new();
        if let Some(v) = env.get("RUNVANE_HOST").cloned() {
            self.host = v.clone();
            applied.insert("RUNVANE_HOST", v);
        }
        if let Some(v) = env.get("RUNVANE_HTTP_PORT").cloned() {
            self.http_port = parse_required("RUNVANE_HTTP_PORT", &v)?;
            applied.insert("RUNVANE_HTTP_PORT", v);
        }
        if let Some(v) = env.get("RUNVANE_GRPC_PORT").cloned() {
            self.grpc_port = parse_required("RUNVANE_GRPC_PORT", &v)?;
            applied.insert("RUNVANE_GRPC_PORT", v);
        }
        if let Some(v) = env.get("RUNVANE_WORKERS").cloned() {
            self.workers = parse_required("RUNVANE_WORKERS", &v)?;
            applied.insert("RUNVANE_WORKERS", v);
        }
        if let Some(v) = env.get("RUNVANE_SQLITE_PATH").cloned() {
            self.sqlite_path = Some(v.clone());
            applied.insert("RUNVANE_SQLITE_PATH", v);
        }
        if let Some(v) = env.get("RUNVANE_LOG_FILTER").cloned() {
            self.log_filter = v.clone();
            applied.insert("RUNVANE_LOG_FILTER", v);
        }
        if let Some(v) = env.get("RUNVANE_AUTH_TOKEN").cloned() {
            self.auth_token = Some(v.clone());
            applied.insert("RUNVANE_AUTH_TOKEN", v);
        }
        if let Some(v) = env.get("RUNVANE_LEASE_MS").cloned() {
            self.lease_ms = parse_required("RUNVANE_LEASE_MS", &v)?;
            applied.insert("RUNVANE_LEASE_MS", v);
        }
        if let Some(v) = env.get("RUNVANE_REAP_MS").cloned() {
            self.reap_interval_ms = parse_required("RUNVANE_REAP_MS", &v)?;
            applied.insert("RUNVANE_REAP_MS", v);
        }
        tracing::debug!(overrides = ?applied, "applied runvane environment overrides");
        self.validate()?;
        Ok(())
    }

    /// Validates field ranges; surfaces obvious misconfigurations early.
    pub fn validate(&self) -> Result<(), RunvaneError> {
        if self.http_port == 0 {
            return Err(RunvaneError::Config("http_port must not be 0".into()));
        }
        if self.grpc_port == 0 {
            return Err(RunvaneError::Config("grpc_port must not be 0".into()));
        }
        if self.lease_ms <= 0 {
            return Err(RunvaneError::Config(
                "lease_ms must be a positive duration".into(),
            ));
        }
        if self.reap_interval_ms <= 0 {
            return Err(RunvaneError::Config(
                "reap_interval_ms must be a positive duration".into(),
            ));
        }
        Ok(())
    }

    /// The effective bind address for the HTTP listener.
    pub fn http_addr(&self) -> String {
        format!("{}:{}", self.host, self.http_port)
    }

    /// The effective bind address for the gRPC listener.
    pub fn grpc_addr(&self) -> String {
        format!("{}:{}", self.host, self.grpc_port)
    }
}

fn parse_required<T: std::str::FromStr>(name: &str, raw: &str) -> Result<T, RunvaneError> {
    raw.parse::<T>().map_err(|_| {
        RunvaneError::Config(format!("{name} ({raw:?}) does not parse as the expected type"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_usable() {
        let cfg = Config::defaults();
        cfg.validate().unwrap();
        assert_eq!(cfg.workers, 4);
        assert_eq!(cfg.http_addr(), "127.0.0.1:8080");
    }

    #[test]
    fn empty_toml_is_defaults() {
        let cfg = Config::from_toml_str("").unwrap();
        assert_eq!(cfg, Config::defaults());
    }

    #[test]
    fn toml_overrides_fields() {
        let cfg = Config::from_toml_str(
            r#"
            host = "0.0.0.0"
            http_port = 9000
            workers = 2
            log_filter = "debug"
            sqlite_path = "runvane.db"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.http_port, 9000);
        assert_eq!(cfg.workers, 2);
        assert_eq!(cfg.log_filter, "debug");
        assert_eq!(cfg.sqlite_path.as_deref(), Some("runvane.db"));
        assert_eq!(cfg.grpc_port, DEFAULT_GRPC_PORT);
    }

    #[test]
    fn unknown_keys_fail_loudly() {
        let err = Config::from_toml_str("hst = \"oops\"\nhttp_pott = 1\n").unwrap_err();
        assert!(
            matches!(err, RunvaneError::Config(_)),
            "expected config error, got {err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("http_pott"), "missing key name in {msg}");
        assert!(msg.contains("hst"), "missing key name in {msg}");
    }

    #[test]
    fn malformed_toml_is_rejected() {
        let err = Config::from_toml_str("http_port = ]").unwrap_err();
        assert!(matches!(err, RunvaneError::Config(_)));
    }

    #[test]
    fn validation_rejects_zero_ports_and_nonpositive_durations() {
        let mut bad = Config::defaults();
        bad.http_port = 0;
        assert!(bad.validate().is_err());
        let mut bad = Config::defaults();
        bad.lease_ms = 0;
        assert!(bad.validate().is_err());
        let mut bad = Config::defaults();
        bad.reap_interval_ms = -5;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn env_overrides_win_over_defaults() {
        let mut env = BTreeMap::new();
        env.insert("RUNVANE_HOST".to_owned(), "0.0.0.0".to_owned());
        env.insert("RUNVANE_HTTP_PORT".to_owned(), "9123".to_owned());
        env.insert("RUNVANE_WORKERS".to_owned(), "7".to_owned());
        env.insert("RUNVANE_LEASE_MS".to_owned(), "15000".to_owned());
        let mut cfg = Config::defaults();
        cfg.apply_env_map(&env).unwrap();
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.http_port, 9123);
        assert_eq!(cfg.workers, 7);
        assert_eq!(cfg.lease_ms, 15000);
        assert_eq!(cfg.grpc_port, DEFAULT_GRPC_PORT);
    }

    #[test]
    fn env_missing_fields_stay_default() {
        let env = BTreeMap::new();
        let mut cfg = Config::defaults();
        cfg.apply_env_map(&env).unwrap();
        assert_eq!(cfg, Config::defaults());
    }

    #[test]
    fn env_bad_value_is_a_typed_error() {
        let mut env = BTreeMap::new();
        env.insert("RUNVANE_HTTP_PORT".to_owned(), "not-a-port".to_owned());
        let mut cfg = Config::defaults();
        let err = cfg.apply_env_map(&env).unwrap_err();
        assert!(
            matches!(err, RunvaneError::Config(_)),
            "expected config error, got {err:?}"
        );
        assert!(err.to_string().contains("RUNVANE_HTTP_PORT"));
    }

    #[test]
    fn env_can_clear_default_fields() {
        let mut env = BTreeMap::new();
        env.insert("RUNVANE_AUTH_TOKEN".to_owned(), "sekrit".to_owned());
        let mut cfg = Config::defaults();
        cfg.apply_env_map(&env).unwrap();
        assert_eq!(cfg.auth_token.as_deref(), Some("sekrit"));
    }
}