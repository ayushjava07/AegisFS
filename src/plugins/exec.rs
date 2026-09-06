//! Production task execution handlers: HTTP requests and bounded subprocess scripts.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value as Json};

use crate::domain::status::FailureKind;
use crate::plugins::handler::{Handler, HandlerError, HandlerResult, TaskContext};

/// Outbound HTTP request execution handler (`runvane.http`).
///
/// Sends synchronous HTTP/1.1 requests to web services, validating response
/// codes and capturing response payloads for downstream task consumption.
#[derive(Debug, Clone)]
pub struct HttpTaskHandler {
    default_timeout: Duration,
}

impl Default for HttpTaskHandler {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(10),
        }
    }
}

impl HttpTaskHandler {
    /// Creates an HTTP task handler with a custom default timeout.
    pub fn new(default_timeout: Duration) -> Self {
        Self { default_timeout }
    }
}

impl Handler for HttpTaskHandler {
    fn execute(&self, ctx: TaskContext<'_>) -> Result<HandlerResult, HandlerError> {
        if ctx.is_cancelled() {
            return Err(HandlerError::cancelled(
                "http task cancelled before execution",
            ));
        }

        let url_str = ctx
            .input
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| HandlerError::permanent("missing required 'url' string parameter"))?;

        let method = ctx
            .input
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("GET")
            .to_uppercase();

        let timeout_ms = ctx
            .input
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .map(Duration::from_millis)
            .unwrap_or(self.default_timeout);

        let parsed_url = url::Url::parse(url_str)
            .map_err(|e| HandlerError::permanent(format!("invalid url '{url_str}': {e}")))?;

        if parsed_url.scheme() != "http" {
            return Err(HandlerError::permanent(
                "only plain http:// URLs are supported by built-in http handler",
            ));
        }

        let host = parsed_url
            .host_str()
            .ok_or_else(|| HandlerError::permanent("url has no host component"))?;
        let port = parsed_url.port().unwrap_or(80);
        let path = if parsed_url.path().is_empty() {
            "/"
        } else {
            parsed_url.path()
        };
        let query = parsed_url
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        let target = format!("{path}{query}");

        let body_str = match ctx.input.get("body") {
            Some(Json::String(s)) => s.clone(),
            Some(v) if !v.is_null() => serde_json::to_string(v).unwrap_or_default(),
            _ => String::new(),
        };

        let mut stream = TcpStream::connect_timeout(
            &format!("{host}:{port}")
                .parse()
                .map_err(|e| HandlerError::permanent(format!("cannot resolve host: {e}")))?,
            timeout_ms,
        )
        .map_err(|e| {
            HandlerError::transient(format!("tcp connect failed to {host}:{port}: {e}"))
        })?;

        stream
            .set_read_timeout(Some(timeout_ms))
            .map_err(|e| HandlerError::transient(format!("set read timeout failed: {e}")))?;
        stream
            .set_write_timeout(Some(timeout_ms))
            .map_err(|e| HandlerError::transient(format!("set write timeout failed: {e}")))?;

        let mut request = format!(
            "{method} {target} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nUser-Agent: runvane-http-worker/1.0\r\n"
        );

        if !body_str.is_empty() {
            request.push_str(&format!("Content-Length: {}\r\n", body_str.len()));
            request.push_str("Content-Type: application/json\r\n");
        }
        request.push_str("\r\n");
        request.push_str(&body_str);

        stream
            .write_all(request.as_bytes())
            .map_err(|e| HandlerError::transient(format!("failed to write http request: {e}")))?;

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|e| HandlerError::transient(format!("failed to read response: {e}")))?;

        let response_str = String::from_utf8_lossy(&response);
        let mut lines = response_str.lines();
        let status_line = lines
            .next()
            .ok_or_else(|| HandlerError::transient("empty http response"))?;

        let parts: Vec<&str> = status_line.split_whitespace().collect();
        if parts.len() < 2 {
            return Err(HandlerError::transient(format!(
                "malformed http status line: {status_line}"
            )));
        }

        let status_code: u16 = parts[1].parse().map_err(|_| {
            HandlerError::transient(format!("invalid status code in {status_line}"))
        })?;

        // Find blank line dividing headers from body
        let body_content = if let Some(idx) = response_str.find("\r\n\r\n") {
            &response_str[idx + 4..]
        } else if let Some(idx) = response_str.find("\n\n") {
            &response_str[idx + 2..]
        } else {
            ""
        };

        let parsed_body: Json = serde_json::from_str(body_content)
            .unwrap_or_else(|_| Json::String(body_content.to_owned()));

        if status_code >= 500 {
            return Err(HandlerError::transient(format!(
                "http server error status {status_code}: {body_content}"
            )));
        }

        if status_code >= 400 {
            return Err(HandlerError::permanent(format!(
                "http client error status {status_code}: {body_content}"
            )));
        }

        Ok(HandlerResult {
            output: json!({
                "status_code": status_code,
                "body": parsed_body,
            }),
        })
    }

    fn description(&self) -> &'static str {
        "executes outbound HTTP requests to external or loopback endpoints"
    }
}

/// Local subprocess execution handler (`runvane.script`).
///
/// Runs command scripts with bounded execution time, capturing stdout and stderr.
#[derive(Debug, Clone, Default)]
pub struct ScriptTaskHandler;

impl Handler for ScriptTaskHandler {
    fn execute(&self, ctx: TaskContext<'_>) -> Result<HandlerResult, HandlerError> {
        if ctx.is_cancelled() {
            return Err(HandlerError::cancelled(
                "script execution cancelled before start",
            ));
        }

        let command = ctx
            .input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| HandlerError::permanent("missing required 'command' string"))?;

        let args: Vec<String> = ctx
            .input
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        let mut cmd = Command::new(command);
        cmd.args(&args);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Environment variables
        if let Some(env_obj) = ctx.input.get("env").and_then(|v| v.as_object()) {
            for (k, v) in env_obj {
                if let Some(s) = v.as_str() {
                    cmd.env(k, s);
                }
            }
        }

        let output = cmd
            .output()
            .map_err(|e| HandlerError::permanent(format!("failed to spawn '{command}': {e}")))?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            return Err(HandlerError {
                kind: FailureKind::Rejected,
                message: format!("command '{command}' exited with code {exit_code}: {stderr}"),
            });
        }

        Ok(HandlerResult {
            output: json!({
                "exit_code": exit_code,
                "stdout": stdout,
                "stderr": stderr,
            }),
        })
    }

    fn description(&self) -> &'static str {
        "executes local subprocess commands with captured stdout and stderr"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::handler::CancellationToken;

    fn make_ctx<'a>(input: &'a Json) -> TaskContext<'a> {
        TaskContext {
            tenant: "acme",
            def_name: "test_pipeline",
            run_id: "rn_test1",
            task_name: "test_task",
            input,
            attempt: 1,
            cancel_token: CancellationToken::new(),
        }
    }

    #[test]
    fn script_handler_executes_echo() {
        let handler = ScriptTaskHandler;
        let input = json!({
            "command": "echo",
            "args": ["hello", "world"]
        });
        let result = handler.execute(make_ctx(&input)).unwrap();
        assert_eq!(result.output["exit_code"], 0);
        let stdout = result.output["stdout"].as_str().unwrap();
        assert!(stdout.contains("hello world"));
    }

    #[test]
    fn script_handler_captures_failure_exit_code() {
        let handler = ScriptTaskHandler;
        let input = json!({
            "command": "false"
        });
        let err = handler.execute(make_ctx(&input)).unwrap_err();
        assert_eq!(err.kind, FailureKind::Rejected);
    }

    #[test]
    fn script_handler_respects_cancellation() {
        let handler = ScriptTaskHandler;
        let input = json!({ "command": "echo" });
        let ctx = make_ctx(&input);
        ctx.cancel_token.cancel();

        let err = handler.execute(ctx).unwrap_err();
        assert_eq!(err.kind, FailureKind::Cancelled);
    }

    #[test]
    fn http_handler_rejects_non_http() {
        let handler = HttpTaskHandler::default();
        let input = json!({ "url": "https://api.github.com" });
        let err = handler.execute(make_ctx(&input)).unwrap_err();
        assert_eq!(err.kind, FailureKind::Rejected);
    }
}
