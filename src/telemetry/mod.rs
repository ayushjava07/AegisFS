use std::collections::HashMap;
use std::time::Instant;

use tracing::{error, info};
use tracing_subscriber::FmtSubscriber;

#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub sampling_rate: f64,
    pub endpoint: Option<String>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sampling_rate: 1.0,
            endpoint: None,
        }
    }
}

pub struct TelemetrySpan {
    name: String,
    start: Instant,
    metadata: HashMap<String, String>,
}

impl TelemetrySpan {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            start: Instant::now(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    pub fn finish(&self, success: bool) {
        let duration = self.start.elapsed();
        record_operation(&self.name, duration, success, &self.metadata);
    }

    pub fn duration_seconds(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

pub fn init_telemetry(config: &TelemetryConfig) {
    if !config.enabled {
        return;
    }

    let subscriber = FmtSubscriber::builder()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
}

pub fn record_operation(
    name: &str,
    duration: std::time::Duration,
    success: bool,
    metadata: &HashMap<String, String>,
) {
    let duration_ms = duration.as_secs_f64() * 1000.0;

    let meta_str: Vec<String> = metadata
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();
    let meta_str = meta_str.join(", ");

    if success {
        info!(
            operation = %name,
            duration_ms = %format!("{:.2}", duration_ms),
            metadata = %meta_str,
            "operation completed successfully",
        );
    } else {
        error!(
            operation = %name,
            duration_ms = %format!("{:.2}", duration_ms),
            metadata = %meta_str,
            "operation failed",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_telemetry_config_default() {
        let config = TelemetryConfig::default();
        assert!(config.enabled);
        assert!((config.sampling_rate - 1.0).abs() < f64::EPSILON);
        assert!(config.endpoint.is_none());
    }

    #[test]
    fn test_telemetry_span_new() {
        let span = TelemetrySpan::new("test_operation");
        assert_eq!(span.name, "test_operation");
    }

    #[test]
    fn test_telemetry_span_with_metadata() {
        let span = TelemetrySpan::new("test")
            .with_metadata("key1", "val1")
            .with_metadata("key2", "val2");

        assert_eq!(span.metadata.get("key1").unwrap(), "val1");
        assert_eq!(span.metadata.get("key2").unwrap(), "val2");
    }

    #[test]
    fn test_telemetry_span_duration() {
        let span = TelemetrySpan::new("timed_op");
        thread::sleep(Duration::from_millis(10));
        assert!(span.duration_seconds() >= 0.01);
    }

    #[test]
    fn test_telemetry_span_duration_accuracy() {
        let span = TelemetrySpan::new("fast_op");
        let d = span.duration_seconds();
        assert!(d < 0.001);
    }

    #[test]
    fn test_record_operation_info_level() {
        let mut metadata = HashMap::new();
        metadata.insert("file".into(), "test.txt".into());

        record_operation("sync_file", Duration::from_millis(150), true, &metadata);
    }

    #[test]
    fn test_record_operation_error_level() {
        let mut metadata = HashMap::new();
        metadata.insert("error".into(), "timeout".into());

        record_operation("sync_file", Duration::from_secs(5), false, &metadata);
    }

    #[test]
    fn test_telemetry_span_finish() {
        let span = TelemetrySpan::new("finish_test").with_metadata("test", "true");
        span.finish(true);
    }

    #[test]
    fn test_telemetry_span_finish_failure() {
        let span = TelemetrySpan::new("finish_fail").with_metadata("reason", "timeout");
        span.finish(false);
    }

    #[test]
    fn test_init_telemetry_disabled() {
        let config = TelemetryConfig {
            enabled: false,
            sampling_rate: 0.0,
            endpoint: None,
        };
        init_telemetry(&config);
    }

    #[test]
    fn test_record_operation_empty_metadata() {
        let metadata = HashMap::new();
        record_operation("no_meta", Duration::from_nanos(500), true, &metadata);
    }

    #[test]
    fn test_telemetry_span_tracks_name() {
        let span = TelemetrySpan::new("my_custom_operation");
        assert_eq!(span.name, "my_custom_operation");
    }

    #[test]
    fn test_multiple_metadata_entries() {
        let span = TelemetrySpan::new("multi")
            .with_metadata("a", "1")
            .with_metadata("b", "2")
            .with_metadata("c", "3");

        assert_eq!(span.metadata.len(), 3);
        assert_eq!(span.metadata.get("c").unwrap(), "3");
    }
}
