/*!
CLI subcommands for OpenTelemetry (OTel) Distributed Tracing & Observability (v4.2).
*/

use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use veloce_ipc::message::OtlpConfigMsg;
use veloce_sdk::VeloceClient;

#[derive(Args, Debug)]
pub struct TraceArgs {
    #[command(subcommand)]
    pub action: Option<TraceAction>,

    /// Trace ID to inspect directly
    #[arg(value_name = "TRACE_ID")]
    pub trace_id: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum TraceAction {
    /// List recent distributed traces across the mesh
    List {
        /// Maximum number of traces to return (default: 20)
        #[arg(short = 'n', long = "limit", default_value = "20")]
        limit: usize,

        /// Filter traces by root service name
        #[arg(short = 's', long = "service")]
        service: Option<String>,
    },

    /// Inspect a trace waterfall and span attributes by Trace ID
    Inspect {
        /// Trace ID (32 hex characters)
        trace_id: String,
    },

    /// Configure OpenTelemetry OTLP/HTTP export endpoint
    Export {
        /// OTLP HTTP endpoint (e.g. http://localhost:4318/v1/traces)
        #[arg(short = 'e', long = "endpoint")]
        endpoint: Option<String>,

        /// Enable exporting
        #[arg(long = "enable")]
        enable: bool,

        /// Disable exporting
        #[arg(long = "disable")]
        disable: bool,
    },

    /// Clear all in-memory recorded traces
    Clear,
}

pub async fn run_trace(client: Arc<Mutex<VeloceClient>>, args: TraceArgs) -> Result<()> {
    match args.action {
        Some(TraceAction::List { limit, service }) => {
            handle_list(client, limit, service).await
        }
        Some(TraceAction::Inspect { trace_id }) => {
            handle_inspect(client, &trace_id).await
        }
        Some(TraceAction::Export { endpoint, enable, disable }) => {
            handle_export(client, endpoint, enable, disable).await
        }
        Some(TraceAction::Clear) => {
            handle_clear(client).await
        }
        None => {
            if let Some(tid) = args.trace_id {
                handle_inspect(client, &tid).await
            } else {
                handle_list(client, 20, None).await
            }
        }
    }
}

pub async fn handle_list(
    client: Arc<Mutex<VeloceClient>>,
    limit: usize,
    service: Option<String>,
) -> Result<()> {
    let mut c = client.lock().await;
    let traces = c.trace_query(Some(limit), service).await?;

    println!("==========================================================================================");
    println!(" OpenTelemetry (OTel) Distributed Traces ({})", traces.len());
    println!("==========================================================================================");
    println!("{:<34} {:<18} {:<24} {:<8} {:<10}", "TRACE ID", "ROOT SERVICE", "OPERATION", "SPANS", "LATENCY");
    println!("------------------------------------------------------------------------------------------");

    if traces.is_empty() {
        println!("  No traces recorded yet. Make HTTP ingress or mesh calls to generate spans.");
    } else {
        for t in traces {
            let status_mark = if t.has_errors { " [ERR]" } else { "" };
            let lat_str = format!("{:.2}ms{}", t.duration_ms, status_mark);
            println!("{:<34} {:<18} {:<24} {:<8} {:<10}",
                t.trace_id,
                truncate_str(&t.root_service, 17),
                truncate_str(&t.root_name, 23),
                t.span_count,
                lat_str,
            );
        }
    }
    println!("==========================================================================================");
    println!("Inspect trace waterfall: 'veloce-run trace inspect <TRACE_ID>'");
    Ok(())
}

pub async fn handle_inspect(
    client: Arc<Mutex<VeloceClient>>,
    trace_id: &str,
) -> Result<()> {
    let mut c = client.lock().await;
    let detail_opt = c.trace_get(trace_id).await?;

    match detail_opt {
        Some(detail) => {
            println!("==========================================================================================");
            println!(" Trace Waterfall: {}", detail.trace_id);
            println!("==========================================================================================");

            let min_start = detail.spans.iter().map(|s| s.start_time_unix_nano).min().unwrap_or(0);
            let max_end = detail.spans.iter().map(|s| s.end_time_unix_nano).max().unwrap_or(0);
            let total_duration_ms = if max_end > min_start {
                (max_end - min_start) as f64 / 1_000_000.0
            } else {
                1.0
            };

            for span in &detail.spans {
                let offset_ms = if span.start_time_unix_nano >= min_start {
                    (span.start_time_unix_nano - min_start) as f64 / 1_000_000.0
                } else {
                    0.0
                };

                let indent = if span.parent_span_id.is_some() { "  └── " } else { "• " };
                let bar = render_waterfall_bar(offset_ms, span.duration_ms, total_duration_ms);

                println!("{}{:<22} [{}] ({:.2}ms)",
                    indent,
                    format!("{}:{}", span.service_name, span.name),
                    span.status_code,
                    span.duration_ms,
                );
                println!("    {} | offset: +{:.2}ms", bar, offset_ms);

                if !span.attributes.is_empty() {
                    let mut attrs_vec: Vec<_> = span.attributes.iter().collect();
                    attrs_vec.sort_by_key(|(k, _)| *k);
                    let attrs_formatted: Vec<String> = attrs_vec
                        .iter()
                        .take(4)
                        .map(|(k, v)| format!("{}={}", k, v))
                        .collect();
                    println!("    tags: [{}]", attrs_formatted.join(", "));
                }
                println!();
            }
            println!("==========================================================================================");
        }
        None => {
            bail!("Trace '{}' not found or expired from memory buffer.", trace_id);
        }
    }

    Ok(())
}

pub async fn handle_export(
    client: Arc<Mutex<VeloceClient>>,
    endpoint: Option<String>,
    enable: bool,
    disable: bool,
) -> Result<()> {
    let mut c = client.lock().await;
    let enabled = if enable { true } else if disable { false } else { true };
    let ep = endpoint.unwrap_or_else(|| "http://localhost:4318/v1/traces".into());

    let config = OtlpConfigMsg {
        endpoint: ep.clone(),
        enabled,
        batch_timeout_secs: 5,
    };

    c.trace_set_otlp_config(config).await?;

    println!("========================================================");
    println!(" OpenTelemetry (OTel) Collector Exporter Updated");
    println!("========================================================");
    println!("  Endpoint: {}", ep);
    println!("  Status:   {}", if enabled { "ENABLED (Streaming spans)" } else { "DISABLED" });
    println!("========================================================");
    Ok(())
}

pub async fn handle_clear(client: Arc<Mutex<VeloceClient>>) -> Result<()> {
    let mut c = client.lock().await;
    c.trace_clear().await?;
    println!("Trace buffer cleared successfully.");
    Ok(())
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}…", &s[..max - 1])
    } else {
        s.to_string()
    }
}

fn render_waterfall_bar(offset_ms: f64, span_ms: f64, total_ms: f64) -> String {
    const BAR_WIDTH: usize = 30;
    if total_ms <= 0.0 {
        return "■".repeat(BAR_WIDTH);
    }

    let start_pos = ((offset_ms / total_ms) * (BAR_WIDTH as f64)).round() as usize;
    let len = ((span_ms / total_ms) * (BAR_WIDTH as f64)).round().max(1.0) as usize;

    let start = start_pos.min(BAR_WIDTH.saturating_sub(1));
    let width = len.min(BAR_WIDTH.saturating_sub(start)).max(1);

    let mut out = String::with_capacity(BAR_WIDTH);
    for i in 0..BAR_WIDTH {
        if i >= start && i < start + width {
            out.push('█');
        } else {
            out.push('░');
        }
    }
    out
}
