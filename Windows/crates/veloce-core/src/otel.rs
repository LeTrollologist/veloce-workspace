/*!
OpenTelemetry (OTel) Native Distributed Tracing & Observability Engine (v4.2).

Provides standard W3C Trace Context propagation, in-memory span ring buffering,
and zero-config OTLP/HTTP JSON exporting for all inter-node IPC and L7 ingress traffic.
*/

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::RwLock;
use rand::Rng;
use veloce_ipc::message::{OtlpConfigMsg, SpanMsg, TraceDetailMsg, TraceSummaryMsg};

const MAX_SPANS: usize = 2000;

#[derive(Debug, Clone)]
pub struct OtelEngine {
    spans: Arc<RwLock<VecDeque<SpanMsg>>>,
    config: Arc<RwLock<OtlpConfigMsg>>,
}

impl OtelEngine {
    pub fn new() -> Self {
        Self {
            spans: Arc::new(RwLock::new(VecDeque::with_capacity(MAX_SPANS))),
            config: Arc::new(RwLock::new(OtlpConfigMsg {
                endpoint: "http://localhost:4318/v1/traces".into(),
                enabled: false,
                batch_timeout_secs: 5,
            })),
        }
    }

    /// Generate a 128-bit W3C Trace ID (32 hex chars).
    pub fn generate_trace_id() -> String {
        let mut rng = rand::thread_rng();
        let bytes: [u8; 16] = rng.gen();
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Generate a 64-bit W3C Span ID (16 hex chars).
    pub fn generate_span_id() -> String {
        let mut rng = rand::thread_rng();
        let bytes: [u8; 8] = rng.gen();
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Parse a W3C `traceparent` header (format: `00-{trace_id}-{parent_id}-{flags}`).
    pub fn parse_traceparent(header: &str) -> Option<(String, String)> {
        let parts: Vec<&str> = header.trim().split('-').collect();
        if parts.len() == 4 && parts[0] == "00" && parts[1].len() == 32 && parts[2].len() == 16 {
            Some((parts[1].to_string(), parts[2].to_string()))
        } else {
            None
        }
    }

    /// Format a W3C `traceparent` header string.
    pub fn format_traceparent(trace_id: &str, span_id: &str) -> String {
        format!("00-{}-{}-01", trace_id, span_id)
    }

    /// Start a new span. Returns a tuple of (trace_id, span_id, start_nano).
    pub fn start_span(
        &self,
        parent_header: Option<&str>,
    ) -> (String, String, Option<String>, u64) {
        let now_nano = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let (trace_id, parent_span_id) = if let Some(hdr) = parent_header {
            if let Some((tid, pid)) = Self::parse_traceparent(hdr) {
                (tid, Some(pid))
            } else {
                (Self::generate_trace_id(), None)
            }
        } else {
            (Self::generate_trace_id(), None)
        };

        let span_id = Self::generate_span_id();
        (trace_id, span_id, parent_span_id, now_nano)
    }

    /// Record a completed span into the trace buffer.
    pub fn record_span(&self, span: SpanMsg) {
        let mut buffer = self.spans.write();
        if buffer.len() >= MAX_SPANS {
            buffer.pop_front();
        }
        buffer.push_back(span);
    }

    /// Query recent traces grouped by trace_id.
    pub fn query_traces(&self, limit: Option<usize>, service_filter: Option<&str>) -> Vec<TraceSummaryMsg> {
        let buffer = self.spans.read();
        let mut trace_map: HashMap<String, Vec<&SpanMsg>> = HashMap::new();

        for span in buffer.iter() {
            if let Some(svc) = service_filter {
                if !span.service_name.eq_ignore_ascii_case(svc) {
                    continue;
                }
            }
            trace_map.entry(span.trace_id.clone()).or_default().push(span);
        }

        let mut summaries = Vec::new();
        for (trace_id, spans) in trace_map {
            if spans.is_empty() {
                continue;
            }

            let root_span = spans.iter().find(|s| s.parent_span_id.is_none()).unwrap_or(&spans[0]);
            let min_start = spans.iter().map(|s| s.start_time_unix_nano).min().unwrap_or(0);
            let max_end = spans.iter().map(|s| s.end_time_unix_nano).max().unwrap_or(0);
            let duration_ms = if max_end > min_start {
                (max_end - min_start) as f64 / 1_000_000.0
            } else {
                root_span.duration_ms
            };

            let has_errors = spans.iter().any(|s| s.status_code == "ERROR");

            summaries.push(TraceSummaryMsg {
                trace_id,
                root_service: root_span.service_name.clone(),
                root_name: root_span.name.clone(),
                span_count: spans.len(),
                duration_ms,
                start_time_unix_nano: min_start,
                has_errors,
            });
        }

        // Sort descending by start time
        summaries.sort_by(|a, b| b.start_time_unix_nano.cmp(&a.start_time_unix_nano));

        let max_results = limit.unwrap_or(50);
        if summaries.len() > max_results {
            summaries.truncate(max_results);
        }

        summaries
    }

    /// Retrieve full trace details with all associated spans.
    pub fn get_trace(&self, trace_id: &str) -> Option<TraceDetailMsg> {
        let buffer = self.spans.read();
        let spans: Vec<SpanMsg> = buffer.iter()
            .filter(|s| s.trace_id == trace_id)
            .cloned()
            .collect();

        if spans.is_empty() {
            None
        } else {
            Some(TraceDetailMsg {
                trace_id: trace_id.to_string(),
                spans,
            })
        }
    }

    /// Clear all stored spans.
    pub fn clear_traces(&self) {
        self.spans.write().clear();
    }

    /// Update OTLP exporter settings.
    pub fn set_otlp_config(&self, config: OtlpConfigMsg) {
        *self.config.write() = config;
    }

    /// Get current OTLP configuration.
    pub fn get_otlp_config(&self) -> OtlpConfigMsg {
        self.config.read().clone()
    }

    /// Render spans into standard OpenTelemetry v1 JSON payload format.
    pub fn format_otlp_json(&self, spans: &[SpanMsg]) -> serde_json::Value {
        let mut scope_spans = Vec::new();
        for span in spans {
            let mut attrs = Vec::new();
            for (k, v) in &span.attributes {
                attrs.push(serde_json::json!({
                    "key": k,
                    "value": { "stringValue": v }
                }));
            }

            scope_spans.push(serde_json::json!({
                "traceId": span.trace_id,
                "spanId": span.span_id,
                "parentSpanId": span.parent_span_id,
                "name": span.name,
                "kind": 1, // SPAN_KIND_INTERNAL
                "startTimeUnixNano": span.start_time_unix_nano.to_string(),
                "endTimeUnixNano": span.end_time_unix_nano.to_string(),
                "attributes": attrs,
                "status": {
                    "code": if span.status_code == "ERROR" { 2 } else { 1 }
                }
            }));
        }

        serde_json::json!({
            "resourceSpans": [{
                "resource": {
                    "attributes": [{
                        "key": "service.name",
                        "value": { "stringValue": "veloce-mesh" }
                    }]
                },
                "scopeSpans": [{
                    "scope": { "name": "veloce-core", "version": "4.2.0" },
                    "spans": scope_spans
                }]
            }]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_traceparent_parsing_and_formatting() {
        let trace_id = "4bf92f3577b34da6a3ce929d0e0e4736";
        let span_id = "00f067aa0ba902b7";
        let header = format!("00-{}-{}-01", trace_id, span_id);

        let parsed = OtelEngine::parse_traceparent(&header).expect("parse traceparent");
        assert_eq!(parsed.0, trace_id);
        assert_eq!(parsed.1, span_id);

        let formatted = OtelEngine::format_traceparent(trace_id, span_id);
        assert_eq!(formatted, header);
    }

    #[test]
    fn test_otel_span_recording_and_query() {
        let engine = OtelEngine::new();
        let (trace_id, root_span_id, _, start_nano) = engine.start_span(None);

        let mut attrs = HashMap::new();
        attrs.insert("http.method".into(), "GET".into());
        attrs.insert("http.target".into(), "/api/v1/orders".into());

        let root_span = SpanMsg {
            trace_id: trace_id.clone(),
            span_id: root_span_id.clone(),
            parent_span_id: None,
            name: "ingress.http_request".into(),
            service_name: "order-service".into(),
            start_time_unix_nano: start_nano,
            end_time_unix_nano: start_nano + 15_000_000,
            duration_ms: 15.0,
            status_code: "OK".into(),
            attributes: attrs,
        };

        engine.record_span(root_span);

        // Child span
        let child_span = SpanMsg {
            trace_id: trace_id.clone(),
            span_id: OtelEngine::generate_span_id(),
            parent_span_id: Some(root_span_id),
            name: "db.query".into(),
            service_name: "order-service".into(),
            start_time_unix_nano: start_nano + 2_000_000,
            end_time_unix_nano: start_nano + 10_000_000,
            duration_ms: 8.0,
            status_code: "OK".into(),
            attributes: HashMap::new(),
        };

        engine.record_span(child_span);

        let traces = engine.query_traces(None, None);
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].trace_id, trace_id);
        assert_eq!(traces[0].span_count, 2);
        assert_eq!(traces[0].root_service, "order-service");

        let detail = engine.get_trace(&trace_id).expect("get trace");
        assert_eq!(detail.spans.len(), 2);

        // OTLP JSON formatting
        let otlp_json = engine.format_otlp_json(&detail.spans);
        assert!(otlp_json.get("resourceSpans").is_some());

        // Clear
        engine.clear_traces();
        assert_eq!(engine.query_traces(None, None).len(), 0);
    }
}
