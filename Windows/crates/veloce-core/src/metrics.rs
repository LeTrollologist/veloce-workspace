/*!
# Prometheus Metrics Exposition Engine for VeloceCore (v3.3)

Renders real-time cluster telemetry, node resource usage, mesh peer stats,
and ingress traffic in standard Prometheus text format (`text/plain; version=0.0.4`).
*/

use std::sync::Arc;
use crate::state::CoreState;

/// Render all current VeloceCore metrics into Prometheus text format.
pub async fn render_prometheus_metrics(state: &Arc<CoreState>) -> String {
    let mut out = String::with_capacity(4096);

    out.push_str("# HELP veloce_build_info VeloceNetwork version and runtime build info\n");
    out.push_str("# TYPE veloce_build_info gauge\n");
    out.push_str(&format!(
        "veloce_build_info{{version=\"{}\"}} 1\n\n",
        env!("CARGO_PKG_VERSION")
    ));

    // ── Node metrics ────────────────────────────────────────────────────────
    let live_nodes = state.node_table().list_live();
    out.push_str("# HELP veloce_nodes_running_total Current number of live supervised nodes\n");
    out.push_str("# TYPE veloce_nodes_running_total gauge\n");
    out.push_str(&format!("veloce_nodes_running_total {}\n\n", live_nodes.len()));

    out.push_str("# HELP veloce_node_info Metadata for each running node\n");
    out.push_str("# TYPE veloce_node_info gauge\n");
    for node in &live_nodes {
        let svc = node.service_name.as_deref().unwrap_or("");
        let health = format!("{:?}", node.health);
        out.push_str(&format!(
            "veloce_node_info{{node_id=\"{}\",app=\"{}\",service=\"{}\",pid=\"{}\",health=\"{}\"}} 1\n",
            node.node_id, node.app_name, svc, node.pid, health
        ));
    }
    out.push('\n');

    // ── Mesh metrics ────────────────────────────────────────────────────────
    if let Some(mesh) = &state.mesh {
        let peers = mesh.peers.read().await;

        out.push_str("# HELP veloce_mesh_peers_connected Number of active P2P mesh peers\n");
        out.push_str("# TYPE veloce_mesh_peers_connected gauge\n");
        out.push_str(&format!("veloce_mesh_peers_connected {}\n\n", peers.len()));

        out.push_str("# HELP veloce_mesh_peer_rtt_ms Round-trip ping latency to connected mesh peer\n");
        out.push_str("# TYPE veloce_mesh_peer_rtt_ms gauge\n");
        for peer in peers.values() {
            let lat = peer.latency_ms.load(std::sync::atomic::Ordering::Relaxed);
            out.push_str(&format!(
                "veloce_mesh_peer_rtt_ms{{peer_id=\"{}\",name=\"{}\"}} {}\n",
                peer.peer_id, peer.peer_name, lat
            ));
        }
        out.push('\n');

        out.push_str("# HELP veloce_mesh_tunnel_tx_bytes Total bytes transmitted over encrypted Noise tunnel\n");
        out.push_str("# TYPE veloce_mesh_tunnel_tx_bytes counter\n");
        out.push_str("# HELP veloce_mesh_tunnel_rx_bytes Total bytes received over encrypted Noise tunnel\n");
        out.push_str("# TYPE veloce_mesh_tunnel_rx_bytes counter\n");
        for peer in peers.values() {
            let (tx, rx) = peer.traffic_snapshot();
            out.push_str(&format!(
                "veloce_mesh_tunnel_tx_bytes{{peer_id=\"{}\",name=\"{}\"}} {}\n",
                peer.peer_id, peer.peer_name, tx
            ));
            out.push_str(&format!(
                "veloce_mesh_tunnel_rx_bytes{{peer_id=\"{}\",name=\"{}\"}} {}\n",
                peer.peer_id, peer.peer_name, rx
            ));
        }
        out.push('\n');

        out.push_str("# HELP veloce_mesh_listen_port P2P mesh listening port\n");
        out.push_str("# TYPE veloce_mesh_listen_port gauge\n");
        out.push_str(&format!("veloce_mesh_listen_port {}\n\n", mesh.listen_port));
    }

    // ── Ingress metrics ─────────────────────────────────────────────────────
    let ingress_rules = state.ingress_router().list_rules().await;
    out.push_str("# HELP veloce_ingress_rules_total Total number of active Layer-7 Ingress routes\n");
    out.push_str("# TYPE veloce_ingress_rules_total gauge\n");
    out.push_str(&format!("veloce_ingress_rules_total {}\n\n", ingress_rules.len()));

    out.push_str("# HELP veloce_ingress_route_info Configured Ingress route details\n");
    out.push_str("# TYPE veloce_ingress_route_info gauge\n");
    for rule in &ingress_rules {
        let tls = if rule.tls_enabled { "true" } else { "false" };
        let default_port = rule.default_port.unwrap_or(0);
        out.push_str(&format!(
            "veloce_ingress_route_info{{host=\"{}\",tls=\"{}\",default_port=\"{}\"}} 1\n",
            rule.host, tls, default_port
        ));
    }
    out.push('\n');

    // ── HPA & Cron metrics ──────────────────────────────────────────────────
    let hpa_policies = state.autoscale().list_policies();
    out.push_str("# HELP veloce_hpa_policies_total Configured Horizontal Process Autoscaler policies\n");
    out.push_str("# TYPE veloce_hpa_policies_total gauge\n");
    out.push_str(&format!("veloce_hpa_policies_total {}\n\n", hpa_policies.len()));

    let cron_jobs = state.cron().list_jobs();
    out.push_str("# HELP veloce_cron_jobs_total Registered scheduled Cron tasks\n");
    out.push_str("# TYPE veloce_cron_jobs_total gauge\n");
    out.push_str(&format!("veloce_cron_jobs_total {}\n\n", cron_jobs.len()));

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_render_prometheus_metrics_format() {
        let temp_dir = std::env::temp_dir().join(format!("vln_test_metrics_{}", uuid::Uuid::new_v4()));
        std::env::set_var("VELOCE_DATA_DIR", &temp_dir);

        let state = Arc::new(CoreState::new().unwrap());
        let metrics = render_prometheus_metrics(&state).await;

        assert!(metrics.contains("veloce_build_info"));
        assert!(metrics.contains("veloce_nodes_running_total"));
        assert!(metrics.contains("veloce_ingress_rules_total"));
        assert!(metrics.contains("veloce_hpa_policies_total"));
        assert!(metrics.contains("veloce_cron_jobs_total"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
