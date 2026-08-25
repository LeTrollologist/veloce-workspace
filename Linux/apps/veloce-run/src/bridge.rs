/*!
CLI subcommands for "Bridge to Cloud" (Unprivileged Kubernetes Remote Telepresence & Traffic Interceptor) (v4.0).
*/

use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use veloce_ipc::message::{BridgeConfigMsg, BridgeInterceptRuleMsg, Capability};
use veloce_sdk::VeloceClient;

#[derive(Args, Debug)]
pub struct BridgeArgs {
    #[command(subcommand)]
    pub action: BridgeAction,
}

#[derive(Subcommand, Debug)]
pub enum BridgeAction {
    /// Connect to a remote Kubernetes bridge peer and tunnel in-cluster DNS and TCP traffic.
    Connect {
        /// Remote Kubernetes peer endpoint, VM3 join code, or mesh hostname
        peer: String,

        /// Remote Kubernetes namespace (default: "default")
        #[arg(short = 'n', long = "namespace", default_value = "default")]
        namespace: String,

        /// Target pod or microservice name
        #[arg(short = 't', long = "target")]
        target: Option<String>,

        /// In-cluster DNS search suffixes to route over the bridge
        #[arg(short = 's', long = "dns-suffix")]
        dns_suffixes: Vec<String>,
    },

    /// Register a traffic interception rule to shadow remote cloud traffic to a local port.
    Intercept {
        /// Remote service or pod name to intercept
        service: String,

        /// Associated bridge session ID (auto-detected if only one session is active)
        #[arg(long = "session")]
        session_id: Option<String>,

        /// Remote port on the Kubernetes pod/service to intercept
        #[arg(short = 'r', long = "remote-port", default_value_t = 8080)]
        remote_port: u16,

        /// Local port in your IDE debugger to receive the intercepted traffic
        #[arg(short = 'l', long = "local-port", default_value_t = 3000)]
        local_port: u16,

        /// Optional HTTP header filter (e.g. "X-Veloce-Intercept: alice" or "X-Debug: true")
        #[arg(short = 'H', long = "header")]
        header: Option<String>,
    },

    /// Run the lightweight, unprivileged bridge sidecar agent inside a remote Kubernetes pod.
    Agent {
        /// Port to listen on inside the Kubernetes pod
        #[arg(short = 'l', long = "listen", default_value_t = 8080)]
        listen: u16,

        /// Target upstream application port in the pod
        #[arg(short = 't', long = "target", default_value_t = 80)]
        target: u16,

        /// Header filter for traffic to shadow/intercept (e.g. "X-Veloce-Intercept")
        #[arg(short = 'H', long = "header")]
        header: Option<String>,
    },

    /// List active cloud bridge sessions and traffic interception rules.
    List,

    /// Disconnect an active cloud bridge session.
    Disconnect {
        /// Bridge session ID to terminate
        session_id: String,
    },
}

pub async fn run_bridge(client: Arc<Mutex<VeloceClient>>, action: BridgeAction) -> Result<()> {
    match action {
        BridgeAction::Connect { peer, namespace, target, dns_suffixes } => {
            let config = BridgeConfigMsg {
                peer: peer.clone(),
                namespace: namespace.clone(),
                target: target.clone(),
                dns_suffixes,
            };

            let mut c = client.lock().await;
            let info = c.bridge_connect(config).await?;

            println!("========================================================");
            println!(" Veloce Cloud Bridge Connected (v4.0)");
            println!("========================================================");
            println!("  Session ID:   {}", info.session_id);
            println!("  Remote Peer:  {}", info.peer);
            println!("  Namespace:    {}", info.namespace);
            if let Some(t) = &info.target {
                println!("  Target:       {}", t);
            }
            println!("  DNS Suffixes: {}", info.dns_suffixes.join(", "));
            println!("========================================================");
            println!("All local processes can now resolve '*.svc.cluster.local' and tunnel TCP traffic.");
            Ok(())
        }

        BridgeAction::Intercept { service, session_id, remote_port, local_port, header } => {
            let mut c = client.lock().await;
            let target_session_id = match session_id {
                Some(s) => s,
                None => {
                    let bridges = c.bridge_list().await?;
                    if bridges.is_empty() {
                        bail!("no active cloud bridge sessions found. Run 'veloce-run bridge connect' first.");
                    }
                    bridges[0].session_id.clone()
                }
            };

            let rule = BridgeInterceptRuleMsg {
                session_id: target_session_id.clone(),
                rule_id: "".into(),
                service_name: service.clone(),
                remote_port,
                local_port,
                header_filter: header.clone(),
            };

            let (sid, rid) = c.bridge_intercept(rule).await?;

            println!("========================================================");
            println!(" Traffic Interception Rule Active (v4.0)");
            println!("========================================================");
            println!("  Rule ID:       {}", rid);
            println!("  Session ID:    {}", sid);
            println!("  Service:       {}", service);
            println!("  Remote Port:   {}", remote_port);
            println!("  Local Target:  localhost:{}", local_port);
            if let Some(h) = header {
                println!("  Header Filter: {}", h);
            } else {
                println!("  Header Filter: (all incoming traffic)");
            }
            println!("========================================================");
            println!("Live staging requests are now intercepted and forwarded to your local debugger.");
            Ok(())
        }

        BridgeAction::Agent { listen, target, header } => {
            println!("========================================================");
            println!(" Veloce Kubernetes Bridge Agent (v4.0)");
            println!("========================================================");
            println!("  Listening:     0.0.0.0:{}", listen);
            println!("  Target Port:   localhost:{}", target);
            if let Some(ref h) = header {
                println!("  Header Filter: {}", h);
            }
            println!("  Mode:          Zero-Root In-Pod Interceptor Sidecar");
            println!("========================================================");
            println!("Waiting for mesh tunnel connections...");

            // Run TCP listener loop
            let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", listen)).await
                .with_context(|| format!("bind bridge agent on 0.0.0.0:{}", listen))?;

            loop {
                let (socket, addr) = match listener.accept().await {
                    Ok(res) => res,
                    Err(e) => {
                        eprintln!("accept error: {e}");
                        continue;
                    }
                };

                let target_port = target;
                tokio::spawn(async move {
                    if let Ok(mut target_stream) = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", target_port)).await {
                        let (mut client_read, mut client_write) = tokio::io::split(socket);
                        let (mut target_read, mut target_write) = tokio::io::split(target_stream);

                        let _ = tokio::join!(
                            tokio::io::copy(&mut client_read, &mut target_write),
                            tokio::io::copy(&mut target_read, &mut client_write),
                        );
                    }
                });
            }
        }

        BridgeAction::List => {
            let mut c = client.lock().await;
            let bridges = c.bridge_list().await?;

            println!("========================================================");
            println!(" Active Cloud Bridge Sessions ({})", bridges.len());
            println!("========================================================");

            if bridges.is_empty() {
                println!("  No active bridge sessions. Run 'veloce-run bridge connect <PEER>' to connect.");
            } else {
                for b in bridges {
                    println!("• Session [{}]", b.session_id);
                    println!("    Peer:        {}", b.peer);
                    println!("    Namespace:   {}", b.namespace);
                    if let Some(t) = b.target {
                        println!("    Target:      {}", t);
                    }
                    println!("    DNS Suffix:  {}", b.dns_suffixes.join(", "));
                    println!("    Intercepts:  {} active", b.active_intercepts.len());
                    for ic in b.active_intercepts {
                        println!("      └─ [{}] {} (remote:{} -> local:{})",
                            ic.rule_id, ic.service_name, ic.remote_port, ic.local_port);
                    }
                    println!();
                }
            }
            println!("========================================================");
            Ok(())
        }

        BridgeAction::Disconnect { session_id } => {
            let mut c = client.lock().await;
            c.bridge_disconnect(&session_id).await?;
            println!("Bridge session '{}' disconnected successfully.", session_id);
            Ok(())
        }
    }
}
