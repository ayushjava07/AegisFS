use std::collections::HashMap;

use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct TraceContext {
    trace_id: String,
    span_id: String,
    parent_span_id: Option<String>,
}

impl TraceContext {
    pub fn new() -> Self {
        Self {
            trace_id: Uuid::new_v4().to_string(),
            span_id: Uuid::new_v4().to_string(),
            parent_span_id: None,
        }
    }

    pub fn from_headers(headers: &HashMap<String, String>) -> Self {
        let trace_id = headers
            .get("x-trace-id")
            .cloned()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let span_id = headers
            .get("x-span-id")
            .cloned()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let parent_span_id = headers.get("x-parent-span-id").cloned();
        Self {
            trace_id,
            span_id,
            parent_span_id,
        }
    }

    pub fn to_headers(&self) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("x-trace-id".into(), self.trace_id.clone());
        headers.insert("x-span-id".into(), self.span_id.clone());
        if let Some(ref parent) = self.parent_span_id {
            headers.insert("x-parent-span-id".into(), parent.clone());
        }
        headers
    }

    pub fn span_id(&self) -> String {
        self.span_id.clone()
    }

    pub fn trace_id(&self) -> String {
        self.trace_id.clone()
    }

    pub fn child_span(&self) -> Self {
        Self {
            trace_id: self.trace_id.clone(),
            span_id: Uuid::new_v4().to_string(),
            parent_span_id: Some(self.span_id.clone()),
        }
    }
}

impl Default for TraceContext {
    fn default() -> Self {
        Self::new()
    }
}
