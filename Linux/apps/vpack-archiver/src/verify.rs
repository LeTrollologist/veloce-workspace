/*!
RFC 8032 Ed25519 Cryptographic Verifier & Digital Signature Tools.
*/
#![allow(dead_code)]

use anyhow::Result;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use crate::archive::VpackArchive;

pub fn verify_signature(archive: &VpackArchive, expected_pubkey: Option<&[u8; 32]>) -> Result<bool> {
    let (pk_bytes, sig_bytes) = match (archive.public_key, archive.signature) {
        (Some(pk), Some(sig)) => (pk, sig),
        _ => return Ok(false),
    };

    if let Some(expected) = expected_pubkey {
        if &pk_bytes != expected {
            return Ok(false);
        }
    }

    let verifying_key = VerifyingKey::from_bytes(&pk_bytes)
        .map_err(|e| anyhow::anyhow!("invalid verifying key: {e}"))?;
    let signature = Signature::from_bytes(&sig_bytes);

    let signed_payload_len = archive.raw_data.len() - 28 - 96;
    let signed_data = &archive.raw_data[..signed_payload_len];

    Ok(verifying_key.verify(signed_data, &signature).is_ok())
}

