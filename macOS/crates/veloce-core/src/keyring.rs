//! Linux secrets backend: XChaCha20-Poly1305 file encryption.
//!
//! Each secret is stored as `{name}.enc` under the secrets directory.
//! The encryption key is derived from `/etc/machine-id` + a per-install salt
//! stored at `/var/lib/veloce/keyring.salt` using standard SHA-256 KDF.
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

// ── Key derivation (SHA-256 FIPS 180-4) ───────────────────────────────────────

fn derive_key() -> Result<Key> {
    let machine_id = read_machine_id()?;
    let salt       = read_or_create_salt()?;

    // Cryptographically secure key derivation using SHA-256 KDF
    let mut material = machine_id.as_bytes().to_vec();
    material.extend_from_slice(b":veloce-keyring-v1:");
    material.extend_from_slice(&salt);

    let key_bytes = sha256_digest(&material);
    Ok(Key::from(key_bytes))
}

fn sha256_digest(data: &[u8]) -> [u8; 32] {
    let mut h = [
        0x6a09e667u32, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];

    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];

    let mut msg = data.to_vec();
    let bit_len = (msg.len() as u64) * 8;
    msg.push(0x80);
    while (msg.len() + 8) % 64 != 0 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i*4], chunk[i*4+1], chunk[i*4+2], chunk[i*4+3]]);
        }
        for i in 16..64 {
            let s0 = w[i-15].rotate_right(7) ^ w[i-15].rotate_right(18) ^ (w[i-15] >> 3);
            let s1 = w[i-2].rotate_right(17) ^ w[i-2].rotate_right(19) ^ (w[i-2] >> 10);
            w[i] = w[i-16].wrapping_add(s0).wrapping_add(w[i-7]).wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut h_var = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h_var.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h_var = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(h_var);
    }

    let mut out = [0u8; 32];
    for (i, val) in h.iter().enumerate() {
        out[i*4..(i+1)*4].copy_from_slice(&val.to_be_bytes());
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_vector() {
        let d = sha256_digest(b"hello world");
        let hex = d.iter().map(|b| format!("{:02x}", b)).collect::<String>();
        assert_eq!(hex, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");
    }
}
