/*!
# veloce-net Ingress — Layer-7 HTTP Reverse Proxy (v2.1)

Provides userspace HTTP routing from standard endpoints (e.g. `127.0.0.1:8080`)
to internal `.vln` backend node ports based on incoming `Host` header and URL path prefix.
*/

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use veloce_ipc::message::IngressRule;

/// Thread-safe table of active Ingress routing rules.
pub struct IngressRouter {
    rules: RwLock<HashMap<String, IngressRule>>,
}

impl IngressRouter {
    pub fn new() -> Self {
        Self {
            rules: RwLock::new(HashMap::new()),
        }
    }

    /// Register or update an ingress rule for `rule.host`.
    pub async fn add_rule(&self, rule: IngressRule) {
        let mut map = self.rules.write().await;
        map.insert(rule.host.to_lowercase(), rule);
    }

    /// Remove an ingress rule by hostname. Returns `true` if found.
    pub async fn remove_rule(&self, host: &str) -> bool {
        let mut map = self.rules.write().await;
        map.remove(&host.to_lowercase()).is_some()
    }

    /// List all currently active ingress rules.
    pub async fn list_rules(&self) -> Vec<IngressRule> {
        let map = self.rules.read().await;
        map.values().cloned().collect()
    }

    /// Match an incoming request's host and path against configured rules.
    ///
    /// Returns `(target_port, optional_rewritten_path)`.
    pub async fn match_route(&self, host: &str, path: &str) -> Option<(u16, Option<String>)> {
        let map = self.rules.read().await;
        let rule = map.get(&host.to_lowercase())?;

        // 1. Check path-specific rules (longest prefix match first)
        let mut matching_paths = rule.paths.clone();
        matching_paths.sort_by(|a, b| b.path_prefix.len().cmp(&a.path_prefix.len()));

        for p in matching_paths {
            if path.starts_with(&p.path_prefix) {
                let rewritten = if p.strip_prefix {
                    let rem = &path[p.path_prefix.len()..];
                    let rem = if !rem.starts_with('/') { format!("/{rem}") } else { rem.to_string() };
                    Some(rem)
                } else {
                    None
                };
                return Some((p.target_port, rewritten));
            }
        }

        // 2. Fall back to default port if configured
        if let Some(port) = rule.default_port {
            return Some((port, None));
        }

        None
    }
}

impl Default for IngressRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Run the Ingress HTTP reverse proxy listener on `{bind_addr}:{port}`.
pub async fn serve(router: Arc<IngressRouter>, bind_addr: &str, port: u16) -> Result<()> {
    let listener = TcpListener::bind(format!("{bind_addr}:{port}"))
        .await
        .with_context(|| format!("bind Ingress HTTP proxy to {bind_addr}:{port}"))?;

    tracing::info!("VeloceNet Ingress HTTP proxy listening on {bind_addr}:{port}");

    loop {
        let (client_stream, client_addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("Ingress accept error: {e}");
                continue;
            }
        };

        let router = Arc::clone(&router);
        tokio::spawn(async move {
            if let Err(e) = handle_ingress_client(client_stream, router).await {
                tracing::debug!("Ingress client ({client_addr}) error: {e}");
            }
        });
    }
}

/// Run the Ingress HTTPS reverse proxy listener on `{bind_addr}:{port}` with TLS termination.
pub async fn serve_tls(
    router: Arc<IngressRouter>,
    bind_addr: &str,
    port: u16,
    tls_manager: Arc<crate::tls::TlsManager>,
) -> Result<()> {
    let listener = TcpListener::bind(format!("{bind_addr}:{port}"))
        .await
        .with_context(|| format!("bind Ingress HTTPS proxy to {bind_addr}:{port}"))?;

    tracing::info!("VeloceNet Ingress HTTPS proxy listening on {bind_addr}:{port} with TLS termination");

    let acceptor = tls_manager.acceptor();

    loop {
        let (client_stream, client_addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("Ingress HTTPS accept error: {e}");
                continue;
            }
        };

        let router = Arc::clone(&router);
        let acceptor = Arc::clone(&acceptor);

        tokio::spawn(async move {
            let tls_stream = match acceptor.accept(client_stream).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!("Ingress TLS handshake ({client_addr}) error: {e}");
                    return;
                }
            };

            if let Err(e) = handle_ingress_tls_client(tls_stream, router).await {
                tracing::debug!("Ingress TLS client ({client_addr}) error: {e}");
            }
        });
    }
}

async fn handle_ingress_tls_client(
    mut client: tokio_rustls::server::TlsStream<TcpStream>,
    router: Arc<IngressRouter>,
) -> Result<()> {
    let mut buf = [0u8; 4096];
    let n = client.read(&mut buf).await.context("read initial request")?;
    if n == 0 {
        return Ok(());
    }

    let raw_req = String::from_utf8_lossy(&buf[..n]);
    let mut lines = raw_req.lines();

    let request_line = match lines.next() {
        Some(l) => l,
        None => return Ok(()),
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");
    let http_ver = parts.next().unwrap_or("HTTP/1.1");

    let host = extract_host_header(&raw_req).unwrap_or_default();

    let route = router.match_route(&host, path).await;
    match route {
        Some((target_port, rewritten_path)) => {
            let mut backend = match TcpStream::connect(format!("127.0.0.1:{target_port}")).await {
                Ok(b) => b,
                Err(e) => {
                    let err_body = format!("{{\"error\":\"upstream backend port {target_port} unreachable: {e}\"}}");
                    let resp = format!(
                        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        err_body.len(), err_body
                    );
                    let _ = client.write_all(resp.as_bytes()).await;
                    return Ok(());
                }
            };

            // If path was rewritten, reconstruct the first line
            if let Some(new_path) = rewritten_path {
                let new_first_line = format!("{method} {new_path} {http_ver}\r\n");
                let rest_of_buf = match raw_req.find("\r\n") {
                    Some(idx) => &buf[idx + 2..n],
                    None => &buf[..0],
                };
                backend.write_all(new_first_line.as_bytes()).await?;
                backend.write_all(rest_of_buf).await?;
            } else {
                backend.write_all(&buf[..n]).await?;
            }

            let _ = tokio::io::copy_bidirectional(&mut client, &mut backend).await;
        }
        None => {
            let not_found_body = format!("{{\"error\":\"no ingress route configured for host '{host}'\"}}");
            let resp = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                not_found_body.len(), not_found_body
            );
            client.write_all(resp.as_bytes()).await?;
        }
    }

    Ok(())
}

async fn handle_ingress_client(
    mut client: TcpStream,
    router: Arc<IngressRouter>,
) -> Result<()> {
    let mut buf = [0u8; 4096];
    let n = client.read(&mut buf).await.context("read initial request")?;
    if n == 0 {
        return Ok(());
    }

    let raw_req = String::from_utf8_lossy(&buf[..n]);
    let mut lines = raw_req.lines();

    let request_line = match lines.next() {
        Some(l) => l,
        None => return Ok(()),
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");
    let http_ver = parts.next().unwrap_or("HTTP/1.1");

    let host = extract_host_header(&raw_req).unwrap_or_default();

    let route = router.match_route(&host, path).await;
    match route {
        Some((target_port, rewritten_path)) => {
            let mut backend = match TcpStream::connect(format!("127.0.0.1:{target_port}")).await {
                Ok(b) => b,
                Err(e) => {
                    let err_body = format!("{{\"error\":\"upstream backend port {target_port} unreachable: {e}\"}}");
                    let resp = format!(
                        "HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        err_body.len(), err_body
                    );
                    let _ = client.write_all(resp.as_bytes()).await;
                    return Ok(());
                }
            };

            // If path was rewritten, reconstruct the first line
            if let Some(new_path) = rewritten_path {
                let new_first_line = format!("{method} {new_path} {http_ver}\r\n");
                let rest_of_buf = match raw_req.find("\r\n") {
                    Some(idx) => &buf[idx + 2..n],
                    None => &buf[..0],
                };
                backend.write_all(new_first_line.as_bytes()).await?;
                backend.write_all(rest_of_buf).await?;
            } else {
                backend.write_all(&buf[..n]).await?;
            }

            let _ = tokio::io::copy_bidirectional(&mut client, &mut backend).await;
        }
        None => {
            let not_found_body = format!("{{\"error\":\"no ingress route configured for host '{host}'\"}}");
            let resp = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                not_found_body.len(), not_found_body
            );
            client.write_all(resp.as_bytes()).await?;
        }
    }

    Ok(())
}

fn extract_host_header(raw_req: &str) -> Option<String> {
    for line in raw_req.lines().skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some(idx) = line.find(':') {
            let header_name = line[..idx].trim();
            if header_name.eq_ignore_ascii_case("host") {
                let val = line[idx + 1..].trim();
                let host = val.split(':').next().unwrap_or(val).to_string();
                if !host.is_empty() {
                    return Some(host);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use veloce_ipc::message::IngressPathRule;

    #[test]
    fn test_extract_host_header_case_insensitive() {
        let req1 = "GET / HTTP/1.1\r\nHost: api.vln:8080\r\nUser-Agent: curl\r\n\r\n";
        assert_eq!(extract_host_header(req1), Some("api.vln".to_string()));

        let req2 = "GET / HTTP/1.1\r\nhost: web.vln\r\n\r\n";
        assert_eq!(extract_host_header(req2), Some("web.vln".to_string()));

        let req3 = "GET / HTTP/1.1\r\nHOST:   service.vln:443  \r\n\r\n";
        assert_eq!(extract_host_header(req3), Some("service.vln".to_string()));

        let req4 = "GET / HTTP/1.1\r\nhOsT: cluster.vln\r\n\r\n";
        assert_eq!(extract_host_header(req4), Some("cluster.vln".to_string()));
    }

    #[tokio::test]
    async fn test_ingress_router_matching() {
        let router = IngressRouter::new();
        let rule = IngressRule {
            host: "api.vln".into(),
            paths: vec![
                IngressPathRule {
                    path_prefix: "/v1/auth".into(),
                    target_port: 4001,
                    strip_prefix: false,
                },
                IngressPathRule {
                    path_prefix: "/v1".into(),
                    target_port: 4000,
                    strip_prefix: true,
                },
            ],
            default_port: Some(8000),
            tls_enabled: false,
            tls_cert_pem: None,
            tls_key_pem: None,
        };
        router.add_rule(rule).await;

        // Longest prefix match
        let (port, rewritten) = router.match_route("api.vln", "/v1/auth/login").await.unwrap();
        assert_eq!(port, 4001);
        assert_eq!(rewritten, None);

        // Strip prefix match
        let (port, rewritten) = router.match_route("api.vln", "/v1/users").await.unwrap();
        assert_eq!(port, 4000);
        assert_eq!(rewritten, Some("/users".into()));

        // Default port fallback
        let (port, rewritten) = router.match_route("api.vln", "/healthz").await.unwrap();
        assert_eq!(port, 8000);
        assert_eq!(rewritten, None);

        // Case insensitivity
        let (port, _) = router.match_route("API.VLN", "/healthz").await.unwrap();
        assert_eq!(port, 8000);

        // Unmatched host
        assert!(router.match_route("other.vln", "/v1").await.is_none());
    }

    #[tokio::test]
    async fn test_ingress_router_remove() {
        let router = IngressRouter::new();
        let rule = IngressRule {
            host: "web.vln".into(),
            paths: vec![],
            default_port: Some(3000),
            tls_enabled: true,
            tls_cert_pem: None,
            tls_key_pem: None,
        };
        router.add_rule(rule).await;
        assert!(router.match_route("web.vln", "/").await.is_some());

        assert!(router.remove_rule("web.vln").await);
        assert!(router.match_route("web.vln", "/").await.is_none());
    }
}
