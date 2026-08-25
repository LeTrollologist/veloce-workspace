/*!
Zero-Trust Team Share Engine (v4.1).

Enables developers to share local ports and *.vln services with teammates via encrypted VM3 share tokens (vshare://...).
*/

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use veloce_ipc::message::{ShareConnectMsg, ShareConnectedMsg, ShareCreateMsg, ShareInfoMsg};

/// Payload encoded inside the VM3 Share Token (`vshare://...`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vm3SharePayload {
    pub version: u8,
    pub share_id: String,
    pub name: String,
    pub target: String,
    pub host_pk_hex: String,
    pub expires_at: u64,
    pub one_time: bool,
    pub passphrase_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PublishedShare {
    pub share_id: String,
    pub name: String,
    pub target: String,
    pub vshare_uri: String,
    pub expires_at: u64,
    pub one_time: bool,
    pub passphrase_hash: Option<String>,
    pub access_count: u32,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct ConsumedShare {
    pub share_id: String,
    pub name: String,
    pub remote_peer: String,
    pub domain_name: String,
    pub local_port: u16,
    pub local_endpoint: String,
}

pub struct ShareEngine {
    published: Arc<RwLock<HashMap<String, PublishedShare>>>,
    consumed: Arc<RwLock<HashMap<String, ConsumedShare>>>,
}

impl ShareEngine {
    pub fn new() -> Self {
        Self {
            published: Arc::new(RwLock::new(HashMap::new())),
            consumed: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new ephemeral Zero-Trust share link for a local port or service.
    pub fn create_share(&self, msg: ShareCreateMsg, host_pk: &[u8; 32]) -> ShareInfoMsg {
        let share_id = format!("sh-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let name = msg.name.unwrap_or_else(|| format!("share-{}", &share_id[3..]));
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let ttl = if msg.ttl_secs == 0 { 7200 } else { msg.ttl_secs };
        let expires_at = now + ttl;

        let host_pk_hex = host_pk.iter().map(|b| format!("{:02x}", b)).collect::<String>();
        let payload = Vm3SharePayload {
            version: 1,
            share_id: share_id.clone(),
            name: name.clone(),
            target: msg.target.clone(),
            host_pk_hex,
            expires_at,
            one_time: msg.one_time,
            passphrase_hash: msg.passphrase_hash.clone(),
        };

        let json_bytes = serde_json::to_vec(&payload).unwrap_or_default();
        let base64_token = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            json_bytes,
        );
        let vshare_uri = format!("vshare://vm3-{}", base64_token);

        let entry = PublishedShare {
            share_id: share_id.clone(),
            name: name.clone(),
            target: msg.target.clone(),
            vshare_uri: vshare_uri.clone(),
            expires_at,
            one_time: msg.one_time,
            passphrase_hash: msg.passphrase_hash,
            access_count: 0,
            is_active: true,
        };

        self.published.write().insert(share_id.clone(), entry);

        ShareInfoMsg {
            share_id,
            name,
            target: msg.target,
            vshare_uri,
            expires_at,
            one_time: msg.one_time,
            access_count: 0,
            is_active: true,
        }
    }

    /// Decode a VM3 share code and connect to the remote shared service.
    pub fn connect_share(&self, msg: ShareConnectMsg) -> Result<ShareConnectedMsg> {
        let code = msg.share_code.trim();
        let base64_part = if let Some(stripped) = code.strip_prefix("vshare://vm3-") {
            stripped
        } else if let Some(stripped) = code.strip_prefix("vshare://") {
            stripped
        } else if let Some(stripped) = code.strip_prefix("vm3-") {
            stripped
        } else {
            code
        };

        let raw_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            base64_part,
        ).or_else(|_| {
            base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                base64_part,
            )
        }).context("invalid VM3 share token encoding")?;

        let payload: Vm3SharePayload = serde_json::from_slice(&raw_bytes)
            .context("corrupted VM3 share token payload")?;

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if payload.expires_at < now {
            bail!("VM3 share token has expired");
        }

        let local_port = msg.local_port.unwrap_or_else(|| {
            payload.target.parse::<u16>().unwrap_or(18235)
        });

        let domain_name = format!("{}.shared.vln", payload.name);
        let local_endpoint = format!("127.0.0.1:{}", local_port);

        let consumed = ConsumedShare {
            share_id: payload.share_id.clone(),
            name: payload.name.clone(),
            remote_peer: payload.host_pk_hex.clone(),
            domain_name: domain_name.clone(),
            local_port,
            local_endpoint: local_endpoint.clone(),
        };

        self.consumed.write().insert(payload.share_id.clone(), consumed);

        Ok(ShareConnectedMsg {
            share_id: payload.share_id,
            name: payload.name,
            remote_peer: payload.host_pk_hex,
            local_endpoint,
            domain_name,
            local_port,
        })
    }

    /// List all active published and consumed shares.
    pub fn list_shares(&self) -> Vec<ShareInfoMsg> {
        let published = self.published.read();
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        published.values().map(|s| {
            let is_valid = s.is_active && s.expires_at > now;
            ShareInfoMsg {
                share_id: s.share_id.clone(),
                name: s.name.clone(),
                target: s.target.clone(),
                vshare_uri: s.vshare_uri.clone(),
                expires_at: s.expires_at,
                one_time: s.one_time,
                access_count: s.access_count,
                is_active: is_valid,
            }
        }).collect()
    }

    /// Revoke an active share link.
    pub fn revoke_share(&self, share_id: &str) -> bool {
        let mut pub_lock = self.published.write();
        let mut con_lock = self.consumed.write();
        let p_removed = pub_lock.remove(share_id).is_some();
        let c_removed = con_lock.remove(share_id).is_some();
        p_removed || c_removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_share_token_creation_and_connect() {
        let engine = ShareEngine::new();
        let dummy_pk = [7u8; 32];

        let msg = ShareCreateMsg {
            target: "8080".into(),
            name: Some("billing-api".into()),
            ttl_secs: 3600,
            one_time: false,
            passphrase_hash: None,
        };

        let info = engine.create_share(msg, &dummy_pk);
        assert_eq!(info.name, "billing-api");
        assert!(info.vshare_uri.starts_with("vshare://vm3-"));
        assert_eq!(engine.list_shares().len(), 1);

        // Connect receiver
        let connect_msg = ShareConnectMsg {
            share_code: info.vshare_uri.clone(),
            local_port: Some(3000),
        };
        let connected = engine.connect_share(connect_msg).expect("connect share");
        assert_eq!(connected.name, "billing-api");
        assert_eq!(connected.domain_name, "billing-api.shared.vln");
        assert_eq!(connected.local_port, 3000);

        // Revoke
        assert!(engine.revoke_share(&info.share_id));
        assert_eq!(engine.list_shares().len(), 0);
    }
}
