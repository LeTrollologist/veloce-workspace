//! Machine identity: x25519 keypair persisted to disk, join-code encoding.
//!
//! The static key pair is used directly with the Noise_IK handshake in `noise.rs`.
//! Ed25519 attestation is planned for v0.5.0; for v0.4.0, identity is the x25519
//! public key embedded in the join code.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::rngs::OsRng;
use uuid::Uuid;
use x25519_dalek::{PublicKey, StaticSecret};

/// A machine's long-term mesh identity.
///
/// Persisted as `veloce-identity.key` (64 bytes: private || public) inside the
/// data directory.  The private key is an x25519 scalar; the public key is its
/// Curve25519 image.  Both are used directly with `snow` for Noise_IK.
pub struct MachineIdentity {
    pub priv_key: [u8; 32],
    pub pub_key:  [u8; 32],
    /// Stable UUID derived from the public key (UUID v5, OID namespace).
    pub machine_id:   Uuid,
    pub machine_name: String,
}

impl MachineIdentity {
    /// Load or generate the identity.
    pub fn load_or_create(data_dir: &Path) -> anyhow::Result<Self> {
        let key_path = data_dir.join("veloce-identity.key");

        let (priv_key, pub_key) = if key_path.exists() {
            let raw = std::fs::read(&key_path)
                .context("read veloce-identity.key")?;
            if raw.len() != 64 {
                bail!("veloce-identity.key is corrupt (expected 64 bytes, got {})", raw.len());
            }
            let mut priv_b = [0u8; 32];
            let mut pub_b  = [0u8; 32];
            priv_b.copy_from_slice(&raw[..32]);
            pub_b.copy_from_slice(&raw[32..]);
            (priv_b, pub_b)
        } else {
            std::fs::create_dir_all(data_dir).context("create data dir")?;
            let secret = StaticSecret::random_from_rng(OsRng);
            let public = PublicKey::from(&secret);
            let priv_b = secret.to_bytes();
            let pub_b  = public.to_bytes();
            let mut raw = [0u8; 64];
            raw[..32].copy_from_slice(&priv_b);
            raw[32..].copy_from_slice(&pub_b);
            std::fs::write(&key_path, &raw)
                .context("write veloce-identity.key")?;
            lock_file_to_owner(&key_path)?;
            (priv_b, pub_b)
        };

        let machine_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, &pub_key);
        let machine_name = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| format!("machine-{}", &machine_id.simple().to_string()[..8]));

        Ok(Self { priv_key, pub_key, machine_id, machine_name })
    }

    /// Build a one-time join code embedding this machine's public key and
    /// the address where the mesh server is listening.
    ///
    /// Format: `VM1:<base64url(pub_key[32] ++ family[1] ++ ip[4|16] ++ port[2] ++ ts[8])>`
    pub fn join_code(&self, listen_addr: SocketAddr) -> String {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut payload: Vec<u8> = Vec::with_capacity(32 + 1 + 16 + 2 + 8);
        payload.extend_from_slice(&self.pub_key);
        match listen_addr.ip() {
            std::net::IpAddr::V4(v4) => {
                payload.push(4u8);
                payload.extend_from_slice(&v4.octets());
            }
            std::net::IpAddr::V6(v6) => {
                payload.push(6u8);
                payload.extend_from_slice(&v6.octets());
            }
        }
        payload.extend_from_slice(&listen_addr.port().to_le_bytes());
        payload.extend_from_slice(&ts.to_le_bytes());
        format!("VM1:{}", URL_SAFE_NO_PAD.encode(&payload))
    }

    /// Decode a join code, returning `(their_x25519_pubkey, their_listen_addr)`.
    pub fn decode_join_code(code: &str) -> anyhow::Result<([u8; 32], SocketAddr)> {
        let b64 = code.strip_prefix("VM1:")
            .context("join code must start with 'VM1:'")?;
        let payload = URL_SAFE_NO_PAD.decode(b64)
            .context("invalid base64 in join code")?;

        if payload.len() < 33 {
            bail!("join code payload too short ({} bytes)", payload.len());
        }
        let mut pub_key = [0u8; 32];
        pub_key.copy_from_slice(&payload[..32]);

        let family = payload[32];
        let addr = match family {
            4 => {
                if payload.len() < 32 + 1 + 4 + 2 {
                    bail!("join code truncated (IPv4)");
                }
                let ip = std::net::Ipv4Addr::new(
                    payload[33], payload[34], payload[35], payload[36],
                );
                let port = u16::from_le_bytes([payload[37], payload[38]]);
                SocketAddr::from((ip, port))
            }
            6 => {
                if payload.len() < 32 + 1 + 16 + 2 {
                    bail!("join code truncated (IPv6)");
                }
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&payload[33..49]);
                let port = u16::from_le_bytes([payload[49], payload[50]]);
                SocketAddr::from((std::net::Ipv6Addr::from(octets), port))
            }
            _ => bail!("unknown address family byte in join code: {family}"),
        };

        Ok((pub_key, addr))
    }
}

// ── File ACL hardening ────────────────────────────────────────────────────────

#[cfg(windows)]
fn lock_file_to_owner(path: &PathBuf) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_READONLY};
    use windows::core::PCWSTR;
    let wide: Vec<u16> = path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_ATTRIBUTE_READONLY)
            .context("SetFileAttributesW on identity key")?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn lock_file_to_owner(path: &PathBuf) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .context("chmod 600 on identity key")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn fake_code(pub_key: &[u8; 32], addr: SocketAddr) -> String {
        let mut payload = Vec::new();
        payload.extend_from_slice(pub_key);
        if let IpAddr::V4(v4) = addr.ip() {
            payload.push(4u8);
            payload.extend_from_slice(&v4.octets());
        }
        payload.extend_from_slice(&addr.port().to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        format!("VM1:{}", URL_SAFE_NO_PAD.encode(&payload))
    }

    #[test]
    fn join_code_round_trip_ipv4() {
        let mut pub_key = [0u8; 32];
        for (i, b) in pub_key.iter_mut().enumerate() { *b = i as u8; }
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 7474);
        let code = fake_code(&pub_key, addr);
        let (k, a) = MachineIdentity::decode_join_code(&code).unwrap();
        assert_eq!(k, pub_key);
        assert_eq!(a, addr);
    }

    #[test]
    fn bad_prefix_rejected() {
        assert!(MachineIdentity::decode_join_code("BAD:abc").is_err());
    }
}
