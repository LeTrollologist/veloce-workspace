/*!
CLI subcommands for Zero-Trust Team Share via VM3 Share Codes (v4.1).
*/

use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::Result;
use clap::{Args, Subcommand};
use veloce_ipc::message::{ShareConnectMsg, ShareCreateMsg};
use veloce_sdk::VeloceClient;

#[derive(Args, Debug)]
pub struct ShareArgs {
    #[command(subcommand)]
    pub action: Option<ShareAction>,

    /// Target port (e.g. 3000) or .vln hostname (e.g. api.vln) when used directly
    #[arg(value_name = "TARGET")]
    pub target: Option<String>,

    /// Friendly name for the share
    #[arg(short = 'n', long = "name")]
    pub name: Option<String>,

    /// Lifetime / TTL (e.g. 2h, 30m, 3600)
    #[arg(short = 't', long = "ttl", default_value = "2h")]
    pub ttl: String,

    /// Revoke the share link immediately after first connection
    #[arg(long = "one-time")]
    pub one_time: bool,

    /// Render terminal ASCII QR Code for mobile scanning
    #[arg(long = "qr")]
    pub qr: bool,
}

#[derive(Subcommand, Debug)]
pub enum ShareAction {
    /// Create and publish a new Zero-Trust share code
    Create {
        /// Target port (e.g. 3000) or .vln hostname (e.g. api.vln)
        target: String,

        /// Friendly name for the share
        #[arg(short = 'n', long = "name")]
        name: Option<String>,

        /// Lifetime / TTL (e.g. 2h, 30m)
        #[arg(short = 't', long = "ttl", default_value = "2h")]
        ttl: String,

        /// Revoke the share link immediately after first connection
        #[arg(long = "one-time")]
        one_time: bool,

        /// Render terminal ASCII QR Code for mobile scanning
        #[arg(long = "qr")]
        qr: bool,
    },

    /// Connect to a remote VM3 share code and expose it locally
    Connect {
        /// The vshare:// URI or VM3 share token
        share_code: String,

        /// Local port override to bind the shared service
        #[arg(short = 'p', long = "port")]
        port: Option<u16>,
    },

    /// List active published and consumed share links
    List,

    /// Revoke an active share link
    Revoke {
        /// Share ID (e.g. sh-12345678)
        share_id: String,
    },
}

pub async fn run_share(client: Arc<Mutex<VeloceClient>>, args: ShareArgs) -> Result<()> {
    match args.action {
        Some(ShareAction::Create { target, name, ttl, one_time, qr }) => {
            handle_create(client, target, name, &ttl, one_time, qr).await
        }
        Some(ShareAction::Connect { share_code, port }) => {
            handle_connect(client, share_code, port).await
        }
        Some(ShareAction::List) => {
            handle_list(client).await
        }
        Some(ShareAction::Revoke { share_id }) => {
            handle_revoke(client, &share_id).await
        }
        None => {
            if let Some(target) = args.target {
                handle_create(client, target, args.name, &args.ttl, args.one_time, args.qr).await
            } else {
                handle_list(client).await
            }
        }
    }
}

pub async fn handle_create(
    client: Arc<Mutex<VeloceClient>>,
    target: String,
    name: Option<String>,
    ttl_str: &str,
    one_time: bool,
    _qr: bool,
) -> Result<()> {
    let ttl_secs = parse_ttl(ttl_str);
    let msg = ShareCreateMsg {
        target: target.clone(),
        name,
        ttl_secs,
        one_time,
        passphrase_hash: None,
    };

    let mut c = client.lock().await;
    let info = c.share_create(msg).await?;

    println!("========================================================");
    println!(" Veloce Zero-Trust Team Share Published (v4.1)");
    println!("========================================================");
    println!("  Share ID:    {}", info.share_id);
    println!("  Name:        {}", info.name);
    println!("  Target:      {}", info.target);
    println!("  Expires in:  {} ({})", ttl_str, format_timestamp(info.expires_at));
    println!("  One-Time:    {}", if info.one_time { "yes (single-use)" } else { "no (reusable)" });
    println!("========================================================");
    println!("\nShare this link with your teammate or client:\n");
    println!("  {}", info.vshare_uri);
    println!("\nTeammate runs:");
    println!("  veloce-run share connect {}", info.vshare_uri);
    println!("========================================================");
    Ok(())
}

pub async fn handle_connect(
    client: Arc<Mutex<VeloceClient>>,
    share_code: String,
    port: Option<u16>,
) -> Result<()> {
    let msg = ShareConnectMsg {
        share_code,
        local_port: port,
    };

    let mut c = client.lock().await;
    let conn = c.share_connect(msg).await?;

    println!("========================================================");
    println!(" Connected to Zero-Trust Team Share (v4.1)");
    println!("========================================================");
    println!("  Service Name:    {}", conn.name);
    println!("  Remote Peer:     {}...", &conn.remote_peer[..16]);
    println!("  Local Endpoint:  http://{}", conn.local_endpoint);
    println!("  Domain Name:     http://{}", conn.domain_name);
    println!("========================================================");
    println!("Service is now accessible directly in your browser and terminal.");
    Ok(())
}

pub async fn handle_list(client: Arc<Mutex<VeloceClient>>) -> Result<()> {
    let mut c = client.lock().await;
    let shares = c.share_list().await?;

    println!("========================================================");
    println!(" Active Team Shares ({})", shares.len());
    println!("========================================================");

    if shares.is_empty() {
        println!("  No active shares. Run 'veloce-run share <PORT>' to publish a local service.");
    } else {
        for s in shares {
            println!("• [{}] {}", s.share_id, s.name);
            println!("    Target:    {}", s.target);
            println!("    Expires:   {}", format_timestamp(s.expires_at));
            println!("    One-Time:  {}", s.one_time);
            println!("    Link:      {}", s.vshare_uri);
            println!();
        }
    }
    println!("========================================================");
    Ok(())
}

pub async fn handle_revoke(client: Arc<Mutex<VeloceClient>>, share_id: &str) -> Result<()> {
    let mut c = client.lock().await;
    c.share_revoke(share_id).await?;
    println!("Share '{}' revoked successfully.", share_id);
    Ok(())
}

fn parse_ttl(s: &str) -> u64 {
    let s = s.trim().to_lowercase();
    if let Some(num) = s.strip_suffix('h') {
        num.parse::<u64>().unwrap_or(2) * 3600
    } else if let Some(num) = s.strip_suffix('m') {
        num.parse::<u64>().unwrap_or(30) * 60
    } else if let Some(num) = s.strip_suffix('s') {
        num.parse::<u64>().unwrap_or(7200)
    } else {
        s.parse::<u64>().unwrap_or(7200)
    }
}

fn format_timestamp(ts: u64) -> String {
    use std::time::SystemTime;
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs();
    if ts > now {
        let diff = ts - now;
        if diff >= 3600 {
            format!("in {}h {}m", diff / 3600, (diff % 3600) / 60)
        } else {
            format!("in {}m", diff / 60)
        }
    } else {
        "expired".into()
    }
}
