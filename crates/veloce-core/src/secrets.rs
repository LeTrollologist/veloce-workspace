/// DPAPI-backed secrets vault for Veloce Compose (v1.2).
///
/// Each secret is stored as an individual `.enc` file under
/// `{data_dir}/secrets/`.  The file content is the raw ciphertext produced
/// by Windows `CryptProtectData` (user-scope, current user only).
///
/// On non-Windows platforms the "encryption" is a no-op passthrough so the
/// Linux build still compiles.  Use a proper vault in production on Linux.
use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use anyhow::{Context, Result};

// ── Public API ─────────────────────────────────────────────────────────────

pub struct SecretsVault {
    secrets_dir: PathBuf,
    names:       RwLock<Vec<String>>,
}

impl SecretsVault {
    /// Scan `data_dir/secrets/` for existing `.enc` files and build the
    /// in-memory name index.
    pub fn new(data_dir: &Path) -> Arc<Self> {
        let secrets_dir = data_dir.join("secrets");
        let _ = std::fs::create_dir_all(&secrets_dir);

        let names: Vec<String> = std::fs::read_dir(&secrets_dir)
            .map(|rd| {
                rd.filter_map(|e| {
                    let e = e.ok()?;
                    let fname = e.file_name().into_string().ok()?;
                    fname.strip_suffix(".enc").map(str::to_owned)
                })
                .collect()
            })
            .unwrap_or_default();

        Arc::new(Self { secrets_dir, names: RwLock::new(names) })
    }

    /// Encrypt `plaintext` with DPAPI and write to `{name}.enc`.
    pub fn set(&self, name: &str, plaintext: &str) -> Result<()> {
        validate_name(name)?;
        let ciphertext = dpapi_protect(plaintext)
            .with_context(|| format!("DPAPI encrypt secret '{name}'"))?;
        let path = self.secret_path(name);
        std::fs::write(&path, &ciphertext)
            .with_context(|| format!("write secret file {:?}", path))?;

        let mut guard = self.names.write();
        if !guard.contains(&name.to_owned()) {
            guard.push(name.to_owned());
        }
        Ok(())
    }

    /// Decrypt and return the plaintext for `name`.
    pub fn get(&self, name: &str) -> Result<String> {
        validate_name(name)?;
        let path = self.secret_path(name);
        let ciphertext = std::fs::read(&path)
            .with_context(|| format!("read secret file {:?}", path))?;
        dpapi_unprotect(&ciphertext)
            .with_context(|| format!("DPAPI decrypt secret '{name}'"))
    }

    /// Delete the secret file and remove from the index.
    pub fn delete(&self, name: &str) -> Result<()> {
        validate_name(name)?;
        let path = self.secret_path(name);
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("delete secret file {:?}", path))?;
        }
        self.names.write().retain(|n| n != name);
        Ok(())
    }

    /// Return a list of known secret names.  Values are never returned.
    pub fn list(&self) -> Vec<String> {
        self.names.read().clone()
    }

    fn secret_path(&self, name: &str) -> PathBuf {
        self.secrets_dir.join(format!("{name}.enc"))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn validate_name(name: &str) -> Result<()> {
    anyhow::ensure!(!name.is_empty(), "secret name must not be empty");
    anyhow::ensure!(
        name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-'),
        "secret name '{}' contains invalid characters (only [A-Za-z0-9_-] allowed)",
        name
    );
    Ok(())
}

// ── DPAPI (Windows) ───────────────────────────────────────────────────────

#[cfg(windows)]
fn dpapi_protect(plaintext: &str) -> Result<Vec<u8>> {
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPT_DATA_BLOB,
    };
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};

    let bytes = plaintext.as_bytes();
    let mut in_blob = CRYPT_DATA_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_DATA_BLOB { cbData: 0, pbData: std::ptr::null_mut() };

    // Use user-scope (dwFlags = 0) so only the same user can decrypt.
    let ok = unsafe {
        CryptProtectData(
            &mut in_blob,
            PCWSTR::null(),
            None,
            None,
            None,
            0,
            &mut out_blob,
        )
    };
    ok.ok().context("CryptProtectData failed")?;

    let out_len = out_blob.cbData as usize;
    let out_ptr = out_blob.pbData;
    let result = unsafe { std::slice::from_raw_parts(out_ptr, out_len).to_vec() };

    // LocalFree the buffer allocated by CryptProtectData
    unsafe { LocalFree(HLOCAL(out_ptr as _)) };

    Ok(result)
}

#[cfg(windows)]
fn dpapi_unprotect(ciphertext: &[u8]) -> Result<String> {
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_DATA_BLOB};
    use windows::Win32::Foundation::{LocalFree, HLOCAL};

    let mut in_blob = CRYPT_DATA_BLOB {
        cbData: ciphertext.len() as u32,
        pbData: ciphertext.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_DATA_BLOB { cbData: 0, pbData: std::ptr::null_mut() };

    let ok = unsafe {
        CryptUnprotectData(
            &mut in_blob,
            None,
            None,
            None,
            None,
            0,
            &mut out_blob,
        )
    };
    ok.ok().context("CryptUnprotectData failed")?;

    let out_len = out_blob.cbData as usize;
    let out_ptr = out_blob.pbData;
    let result = unsafe {
        std::str::from_utf8(std::slice::from_raw_parts(out_ptr, out_len))
            .context("DPAPI output is not valid UTF-8")?
            .to_owned()
    };

    unsafe { LocalFree(HLOCAL(out_ptr as _)) };

    Ok(result)
}

// Stub implementations for non-Windows builds (Linux v2.0 track).
#[cfg(not(windows))]
fn dpapi_protect(plaintext: &str) -> Result<Vec<u8>> {
    Ok(plaintext.as_bytes().to_vec())
}

#[cfg(not(windows))]
fn dpapi_unprotect(ciphertext: &[u8]) -> Result<String> {
    Ok(String::from_utf8_lossy(ciphertext).into_owned())
}
