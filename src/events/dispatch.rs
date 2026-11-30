//! Hook matching and delivery.
//!
//! A definition's [`Hooks`] bind `HookSpec`s to lifecycle triggers. The only
//! nontrivial logic is matching — which specs fire for a given event kind, and
//! whether the spec's `event_filter` admits the event. Delivery itself goes
//! through a [`WebhookSink`], so tests pin behavior with an in-memory sink
//! while the shipped default is a logging sink (and [`HttpSink`] for real
//! loopback webhooks, dependency-free).

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value as Json;

use crate::domain::workflow::{HookSpec, WorkflowDef};
use crate::events::{delivery_id, EventKind, RunEventDoc};

/// A single webhook delivery decision: destination, headers, and payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    /// Stable id for this delivery (dedup on the consumer side).
    pub id: String,
    /// Target URL.
    pub url: String,
    /// Extra headers from the hook spec.
    pub headers: BTreeMap<String, String>,
    /// Serialized event document, JSON.
    pub body_json: String,
}

/// How many deliveries a definition's hooks produced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchStats {
    /// Hook specs that matched this event.
    pub matched: usize,
    /// Specs filtered out by `event_filter`.
    pub filtered: usize,
    /// Specs skipped because they had no webhook URL.
    pub no_url: usize,
}

/// Selects the `HookSpec`s that apply to an event, in definition order.
///
/// Matching rules: the spec's slice (start/success/failure/cancel) selects by
/// kind; within it, `event_filter`, when set, must equal either the event code
/// (e.g. `run.failed`) or the run status name (`failed`).
pub fn match_hooks(def: &WorkflowDef, kind: EventKind) -> (Vec<&HookSpec>, MatchStats) {
    let slice: &[HookSpec] = match kind {
        EventKind::RunStarted => &def.hooks.on_start,
        EventKind::RunSucceeded => &def.hooks.on_success,
        EventKind::RunFailed | EventKind::RunTimedOut => &def.hooks.on_failure,
        EventKind::RunCancelled => &def.hooks.on_cancel,
    };
    let event_code = kind.code();
    let mut stats = MatchStats::default();
    let mut matched = Vec::new();
    for spec in slice {
        match &spec.event_filter {
            Some(filter)
                if filter != event_code && filter != event_code.trim_start_matches("run.") =>
            {
                stats.filtered += 1;
            }
            _ => {
                if spec.webhook_url.is_none() {
                    stats.no_url += 1;
                } else {
                    stats.matched += 1;
                    matched.push(spec);
                }
            }
        }
    }
    (matched, stats)
}

/// Builds the payload document and turns matched specs into deliveries.
pub fn build_deliveries(
    def: &WorkflowDef,
    kind: EventKind,
    ev: &RunEventDoc,
) -> (Vec<Delivery>, MatchStats) {
    let (matched, stats) = match_hooks(def, kind);
    let deliveries = matched
        .into_iter()
        .map(|spec| {
            let url = spec
                .webhook_url
                .as_ref()
                .expect("matched spec always has a webhook url");
            Delivery {
                id: delivery_id(
                    &crate::domain::ids::RunId::from_validated(ev.run_id.clone()),
                    kind,
                    &ev.tenant,
                    &ev.def_name,
                ),
                url: url.clone(),
                headers: spec.headers.clone(),
                body_json: serde_json::to_string(ev).expect("event doc serializes"),
            }
        })
        .collect();
    (deliveries, stats)
}

/// Produces the JSON body for a single delivery (used by tests and sinks).
pub fn body_for(ev: &RunEventDoc) -> Json {
    serde_json::to_value(ev).expect("event doc serializes")
}

/// The outbound delivery mechanism. Sync because deliveries are cheap and the
/// webhook receiver is expected to be a loopback latency away.
pub trait WebhookSink: Send + Sync {
    /// Attempts one delivery. `Ok(())` means accepted; retried later is not an
    /// offer here — the event bus treats a failure as observability-only.
    fn deliver(&self, delivery: &Delivery) -> Result<(), String>;
}

/// A sink that records everything and returns success — the workhorse of the
/// test-suite and a handy audit trail.
#[derive(Debug, Default, Clone)]
pub struct RecordingSink {
    deliveries: Arc<Mutex<Vec<Delivery>>>,
}

impl RecordingSink {
    /// Everything delivered so far, in order.
    pub fn recorded(&self) -> Vec<Delivery> {
        self.deliveries.lock().unwrap().clone()
    }

    /// How many deliveries landed.
    pub fn count(&self) -> usize {
        self.recorded().len()
    }
}

impl WebhookSink for RecordingSink {
    fn deliver(&self, delivery: &Delivery) -> Result<(), String> {
        self.deliveries.lock().unwrap().push(delivery.clone());
        Ok(())
    }
}

/// Operators across runs; sink best-effort with a JSON body and headers.
#[derive(Debug, Default, Clone)]
pub struct LoggingSink;

impl WebhookSink for LoggingSink {
    fn deliver(&self, delivery: &Delivery) -> Result<(), String> {
        tracing::info!(
            delivery = %delivery.id,
            url = %delivery.url,
            status = ?delivery.headers.get("X-Runvane-Status"),
            "webhook delivery"
        );
        Ok(())
    }
}

/// A synchronous HTTP/1.1 POST client for plain `http://` webhook targets.
/// Deliberately tiny and dependency-free: internal webhooks are loopback
/// services, and the transport is not a production-grade outbound client.
#[derive(Debug, Clone)]
pub struct HttpSink {
    /// Connect/read timeout per delivery.
    pub timeout: Duration,
}

impl Default for HttpSink {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(3),
        }
    }
}

impl HttpSink {
    /// True when the URL is a plain http URL this sink can speak to.
    pub fn supports(url: &str) -> bool {
        url.starts_with("http://")
    }

    fn request(&self, delivery: &Delivery) -> Result<(), String> {
        let url = url::Url::parse(&delivery.url)
            .map_err(|e| format!("invalid webhook url {}: {e}", delivery.url))?;
        if url.scheme() != "http" {
            return Err(format!("unsupported webhook scheme {:?}", url.scheme()));
        }
        let host = url
            .host_str()
            .ok_or_else(|| format!("webhook url {} has no host", delivery.url))?;
        let port = url.port().unwrap_or(80);
        let path = if url.path().is_empty() {
            "/"
        } else {
            url.path()
        };
        let mut stream = TcpStream::connect((host, port))
            .map_err(|e| format!("cannot connect to {host}:{port}: {e}"))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|e| e.to_string())?;

        let mut request = String::new();
        request.push_str(&format!("POST {path} HTTP/1.1\r\n"));
        request.push_str(&format!("Host: {host}\r\n"));
        request.push_str("User-Agent: runvane-events/1\r\n");
        request.push_str("Content-Type: application/json\r\n");
        request.push_str("Accept: application/json\r\n");
        for (k, v) in &delivery.headers {
            request.push_str(&format!("{k}: {v}\r\n"));
        }
        request.push_str(&format!("Content-Length: {}\r\n", delivery.body_json.len()));
        request.push_str("Connection: close\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(delivery.body_json.as_bytes()))
            .map_err(|e| format!("write to {host}:{port} failed: {e}"))?;

        // Read the status line; the body is dropped by design (dedup via id).
        let mut response = Vec::new();
        stream
            .take(4 * 1024)
            .read_to_end(&mut response)
            .map_err(|e| format!("read from {host}:{port} failed: {e}"))?;
        let status_line = {
            let text = String::from_utf8_lossy(&response);
            text.lines().next().unwrap_or_default().to_owned()
        };
        if status_line.contains(" 2") || status_line.contains(" 3") {
            Ok(())
        } else {
            Err(format!("webhook {host}:{port} responded {status_line:?}"))
        }
    }
}

impl WebhookSink for HttpSink {
    fn deliver(&self, delivery: &Delivery) -> Result<(), String> {
        if !Self::supports(&delivery.url) {
            return Err(format!("unsupported webhook url {}", delivery.url));
        }
        self.request(delivery)
    }
}

/// Fires the deliveries for a matched event through the configured sink.
///
/// The `delivered` counter is a cheap observability probe for the watcher; it
/// is not a retry ledger — a transient failure is surfaced via tracing only, so
/// a flaky webhook can never wedge the scheduler thread.
#[derive(Clone)]
pub struct Dispatcher {
    sink: Arc<dyn WebhookSink>,
    delivered: Arc<AtomicU64>,
}

impl std::fmt::Debug for Dispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dispatcher")
            .field("delivered_total", &self.delivered_total())
            .finish_non_exhaustive()
    }
}

impl Dispatcher {
    /// A dispatcher bound to a sink.
    pub fn new(sink: Arc<dyn WebhookSink>) -> Self {
        Self {
            sink,
            delivered: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Dispatches the event through the definition's hooks.
    pub fn dispatch(&self, def: &WorkflowDef, kind: EventKind, ev: &RunEventDoc) -> MatchStats {
        let (deliveries, stats) = build_deliveries(def, kind, ev);
        for delivery in &deliveries {
            match self.sink.deliver(delivery) {
                Ok(()) => {
                    self.delivered.fetch_add(1, Ordering::Relaxed);
                }
                Err(err) => {
                    tracing::warn!(delivery = %delivery.id, url = %delivery.url, error = %err, "webhook delivery failed");
                }
            }
        }
        stats
    }

    /// Total deliveries that succeeded since construction.
    pub fn delivered_total(&self) -> u64 {
        self.delivered.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{HookSpec, Hooks};

    fn def_with_hooks(hooks: Hooks) -> WorkflowDef {
        WorkflowDef {
            name: "ship".to_owned(),
            hooks,
            ..WorkflowDef::default()
        }
    }

    fn spec(url: &str, filter: Option<&str>) -> HookSpec {
        HookSpec {
            webhook_url: Some(url.to_owned()),
            event_filter: filter.map(str::to_owned),
            headers: BTreeMap::new(),
        }
    }

    fn event(kind: EventKind) -> RunEventDoc {
        RunEventDoc {
            kind: kind.code().to_owned(),
            run_id: "rn_abc".to_owned(),
            tenant: "acme".to_owned(),
            def_name: "ship".to_owned(),
            def_version: 3,
            run_number: 7,
            status: match kind {
                EventKind::RunStarted => "running".to_owned(),
                EventKind::RunSucceeded => "succeeded".to_owned(),
                _ => "failed".to_owned(),
            },
            started_at_ms: Some(100),
            finished_at_ms: Some(200),
            attempts: 1,
            error: None,
            output: None,
            created_at_ms: 200,
        }
    }

    #[test]
    fn match_selects_the_right_slice() {
        let def = def_with_hooks(Hooks {
            on_start: vec![spec("http://a/start", None)],
            on_success: vec![spec("http://a/success", None), spec("http://a/s2", None)],
            on_failure: vec![spec("http://a/fail", None)],
            on_cancel: vec![spec("http://a/cancel", None)],
        });
        let (start, _) = match_hooks(&def, EventKind::RunStarted);
        assert_eq!(start.len(), 1);
        let (ok, _) = match_hooks(&def, EventKind::RunSucceeded);
        assert_eq!(ok.len(), 2);
        let (fail, _) = match_hooks(&def, EventKind::RunFailed);
        assert_eq!(fail.len(), 1);
        let (timeout, _) = match_hooks(&def, EventKind::RunTimedOut);
        assert_eq!(timeout.len(), 1, "timed out also uses on_failure");
        let (cancel, _) = match_hooks(&def, EventKind::RunCancelled);
        assert_eq!(cancel.len(), 1);
    }

    #[test]
    fn event_filter_admits_code_or_status_name() {
        let def = def_with_hooks(Hooks {
            on_failure: vec![
                spec("http://a/x", Some("run.failed")),
                spec("http://a/y", Some("failed")),
                spec("http://a/z", Some("run.succeeded")),
            ],
            ..Hooks::default()
        });
        let (matched, stats) = match_hooks(&def, EventKind::RunFailed);
        assert_eq!(matched.len(), 2);
        assert_eq!(stats.filtered, 1);
    }

    #[test]
    fn no_url_specs_are_skipped_not_matched() {
        let no_url = HookSpec {
            webhook_url: None,
            ..HookSpec::default()
        };
        let def = def_with_hooks(Hooks {
            on_success: vec![no_url],
            ..Hooks::default()
        });
        let (matched, stats) = match_hooks(&def, EventKind::RunSucceeded);
        assert!(matched.is_empty());
        assert_eq!(stats.no_url, 1);
    }

    #[test]
    fn deliveries_carry_a_stable_dedup_id() {
        let ev = event(EventKind::RunFailed);
        let def = def_with_hooks(Hooks {
            on_failure: vec![spec("http://a/fail", None)],
            ..Hooks::default()
        });
        let (deliveries, stats) = build_deliveries(&def, EventKind::RunFailed, &ev);
        assert_eq!(stats.matched, 1);
        assert_eq!(deliveries.len(), 1);
        // Same run+kind yields the same id every time (dedup contract).
        let (again, _) = build_deliveries(&def, EventKind::RunFailed, &ev);
        assert_eq!(deliveries[0].id, again[0].id);
        // A different kind reaches a different slice (no on_start hooks here).
        let start = event(EventKind::RunStarted);
        let (start_d, _) = build_deliveries(&def, EventKind::RunStarted, &start);
        assert!(
            start_d.is_empty(),
            "kind slices are disjoint by construction"
        );
        // The body round-trips to the event document.
        let body: RunEventDoc = serde_json::from_str(&deliveries[0].body_json).unwrap();
        assert_eq!(body.kind, "run.failed");
        assert_eq!(body.run_id, "rn_abc");
    }

    #[test]
    fn recording_sink_buffers_in_order() {
        let sink = RecordingSink::default();
        let dispatcher = Dispatcher::new(Arc::new(sink.clone()));
        let def = def_with_hooks(Hooks {
            on_success: vec![spec("http://a/success", None)],
            ..Hooks::default()
        });
        let ev = event(EventKind::RunSucceeded);
        let stats = dispatcher.dispatch(&def, EventKind::RunSucceeded, &ev);
        assert_eq!(stats.matched, 1);
        assert_eq!(dispatcher.delivered_total(), 1);
        let recorded = sink.recorded();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].url, "http://a/success");
        let body: RunEventDoc = serde_json::from_str(&recorded[0].body_json).unwrap();
        assert_eq!(body.kind, "run.succeeded");
    }

    #[test]
    fn empty_hooks_dispatch_nothing() {
        let sink = RecordingSink::default();
        let dispatcher = Dispatcher::new(Arc::new(sink.clone()));
        let def = def_with_hooks(Hooks::default());
        let ev = event(EventKind::RunSucceeded);
        let stats = dispatcher.dispatch(&def, EventKind::RunSucceeded, &ev);
        assert_eq!(stats.matched, 0);
        assert_eq!(sink.count(), 0);
        assert_eq!(dispatcher.delivered_total(), 0);
    }

    #[test]
    fn http_sink_says_what_it_supports() {
        assert!(HttpSink::supports("http://localhost:8080/hook"));
        assert!(!HttpSink::supports("https://localhost:8080/hook"));
        assert!(!HttpSink::supports("not a url"));
    }

    #[test]
    fn http_sink_rejects_non_http_and_bogus_urls() {
        let sink = HttpSink::default();
        for url in ["https://example.com/hook", "ftp://x", "garbage", "http://"] {
            let delivery = Delivery {
                id: "d1".into(),
                url: url.to_owned(),
                headers: BTreeMap::new(),
                body_json: "{}".into(),
            };
            assert!(sink.request(&delivery).is_err(), "should reject {url}");
        }
    }

    /// A tiny one-shot HTTP/1.1 responder on a thread. Reads the request
    /// (through the declared Content-Length) and answers 204.
    #[test]
    fn http_sink_posts_to_a_loopback_receiver() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let served = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let served2 = std::sync::Arc::clone(&served);
        let thread_handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            // Read until header end + declared body length fully arrive.
            loop {
                let n = stream.read(&mut chunk).unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf);
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let content_length: usize = text
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse().ok())
                        })
                        .unwrap_or(0);
                    if buf.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
            }
            *served2.lock().unwrap() = buf.clone();
            let response = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n";
            stream.write_all(response.as_bytes()).unwrap();
        });

        let sink = HttpSink::default();
        let delivery = Delivery {
            id: "d-http".into(),
            url: format!("http://{addr}/hooks/pipeline"),
            headers: BTreeMap::from([("X-Delivery".into(), "1".into())]),
            body_json: r#"{"kind":"run.succeeded"}"#.into(),
        };
        sink.request(&delivery).unwrap();
        thread_handle.join().unwrap();

        let request_body = {
            let served = served.lock().unwrap();
            String::from_utf8_lossy(&served).into_owned()
        };
        assert!(request_body.starts_with("POST /hooks/pipeline HTTP/1.1"));
        assert!(request_body.contains("Host: "));
        assert!(request_body.contains("X-Delivery: 1"));
        assert!(request_body.contains("Content-Type: application/json"));
        assert!(request_body.contains(r#"{"kind":"run.succeeded"}"#));
    }

    #[test]
    fn http_sink_reports_hard_failures() {
        let delivery = Delivery {
            id: "d-x".into(),
            url: "http://127.0.0.1:9/hook".into(), // port 9 (discard) refuses
            headers: BTreeMap::new(),
            body_json: "{}".into(),
        };
        let sink = HttpSink::default();
        // Connection may be refused or accepted-then-dropped; both must stop us.
        assert!(sink.request(&delivery).is_err());
    }
}
