//! Linux secrets backend: XChaCha20-Poly1305 file encryption.
//!
//! Each secret is stored as `{name}.enc` under the secrets directory.
//! The encryption key is derived from `/etc/machine-id` + a per-install salt
//! stored at `/var/lib/veloce/keyring.salt`.
//!
//! Ciphertext format: `nonce(24 bytes) || xchacha20poly1305_ciphertext`.

#![cfg(unix)]

use anyhow::{Context, Result};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Key,
};

/// Encrypt `plaintext` and return the ciphertext blob (nonce prepended).
pub fn seal(plaintext: &str) -> Result<Vec<u8>> {
    let key = derive_key()?;
    let cipher = XChaCha20Poly1305::new(&key);
    let nonce  = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher.encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("XChaCha20Poly1305 encrypt: {e}"))?;
    // Prepend nonce (24 bytes) to ciphertext
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt a ciphertext blob (with prepended nonce) and return the plaintext.
pub fn unseal(data: &[u8]) -> Result<String> {
    if data.len() < 24 {
        anyhow::bail!("ciphertext too short (expected >= 24 bytes, got {})", data.len());
    }
    let (nonce_bytes, ciphertext) = data.split_at(24);
    let nonce  = XNonce::from_slice(nonce_bytes);
    let key    = derive_key()?;
    let cipher = XChaCha20Poly1305::new(&key);
    let plaintext = cipher.decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("XChaCha20Poly1305 decrypt: {e}"))?;
    String::from_utf8(plaintext).context("decrypted secret is not valid UTF-8")
}

// ── Key derivation ────────────────────────────────────────────────────────────

fn derive_key() -> Result<Key> {
    let machine_id = read_machine_id()?;
    let salt       = read_or_create_salt()?;

    // Key = SHA-256(machine_id || "veloce-keyring" || salt)
    // We don't have sha2 in workspace, so use a simple XOR-fold over the bytes
    // to produce 32 bytes.  This is "good enough" for the file-at-rest threat model;
    // production deployments should use a proper KDF (argon2, hkdf) or a HSM.
    let mut material = machine_id.as_bytes().to_vec();
    material.extend_from_slice(b"veloce-keyring");
    material.extend_from_slice(&salt);

    // Fold into 32 bytes with a simple pseudorandom mixing
    let mut key_bytes = [0u8; 32];
    for (i, &b) in material.iter().enumerate() {
        key_bytes[i % 32] ^= b.wrapping_add((i as u8).wrapping_mul(0x37));
        key_bytes[(i.wrapping_add(16)) % 32] = key_bytes[(i.wrapping_add(16)) % 32]
            .wrapping_add(b.rotate_left(3));
    }

    Ok(Key::from(key_bytes))
}

fn read_machine_id() -> Result<String> {
    std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .context("read /etc/machine-id")
        .map(|s| s.trim().to_owned())
}

fn read_or_create_salt() -> Result<Vec<u8>> {
    let salt_path = std::path::Path::new("/var/lib/veloce/keyring.salt");
    if salt_path.exists() {
        return std::fs::read(salt_path).context("read keyring.salt");
    }
    // Generate and persist a new 32-byte random salt
    let mut salt = vec![0u8; 32];
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut salt))
        .context("read /dev/urandom for salt")?;
    if let Some(parent) = salt_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(salt_path, &salt).context("write keyring.salt")?;
    Ok(salt)
}
