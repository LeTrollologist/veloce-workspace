/// Named volume registry for Veloce Compose (v1.2).
///
/// Named volumes are directories managed under the Veloce data directory.
/// Bind mounts are host paths validated at mount time.
///
/// The volume catalog is persisted as `veloce-volumes.json` so volumes
/// survive Core restarts.
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use chrono::{DateTime, Utc};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ── On-disk catalog format ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CatalogEntry {
    host_path:  PathBuf,
    created_at: DateTime<Utc>,
}

// ── Public API ─────────────────────────────────────────────────────────────

pub struct VolumeRegistry {
    catalog_path: PathBuf,
    base_dir:     PathBuf,
    volumes:      RwLock<HashMap<String, CatalogEntry>>,
}

impl VolumeRegistry {
    /// Load the catalog from `data_dir/veloce-volumes.json`, creating it if
    /// absent.  Named volumes live under `data_dir/volumes/{name}/`.
    pub fn new(data_dir: &Path) -> Arc<Self> {
        let catalog_path = data_dir.join("veloce-volumes.json");
        let base_dir     = data_dir.join("volumes");

        let volumes = if catalog_path.exists() {
            let text = std::fs::read_to_string(&catalog_path).unwrap_or_default();
            serde_json::from_str::<HashMap<String, CatalogEntry>>(&text)
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        Arc::new(Self {
            catalog_path,
            base_dir,
            volumes: RwLock::new(volumes),
        })
    }

    /// Return the host path for a named volume, creating the directory (and
    /// catalog entry) if it does not already exist.
    pub fn get_or_create(&self, name: &str) -> Result<PathBuf> {
        validate_volume_name(name)?;

        // Fast path — already known
        {
            let guard = self.volumes.read();
            if let Some(entry) = guard.get(name) {
                return Ok(entry.host_path.clone());
            }
        }

        // Create directory
        let host_path = self.base_dir.join(name);
        std::fs::create_dir_all(&host_path)
            .with_context(|| format!("create volume dir {:?}", host_path))?;

        let entry = CatalogEntry { host_path: host_path.clone(), created_at: Utc::now() };
        self.volumes.write().insert(name.to_owned(), entry);
        self.persist();
        Ok(host_path)
    }

    /// Validate a bind-mount host path (must exist; does not need to be a
    /// directory, but usually is).
    pub fn resolve_bind(host_path_str: &str) -> Result<PathBuf> {
        let p = PathBuf::from(host_path_str);
        anyhow::ensure!(
            p.exists(),
            "bind-mount path does not exist: {:?}",
            p
        );
        Ok(p)
    }

    /// Return a snapshot of all known volumes as IPC-ready structs.
    pub fn list(&self) -> Vec<veloce_ipc::message::VolumeEntry> {
        self.volumes
            .read()
            .iter()
            .map(|(name, e)| veloce_ipc::message::VolumeEntry {
                name:       name.clone(),
                host_path:  e.host_path.to_string_lossy().into_owned(),
                created_at: e.created_at,
            })
            .collect()
    }

    // ── Private ───────────────────────────────────────────────────────────

    fn persist(&self) {
        let guard = self.volumes.read();
        match serde_json::to_string_pretty(&*guard) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.catalog_path, json) {
                    tracing::warn!("volume catalog persist failed: {e}");
                }
            }
            Err(e) => tracing::warn!("volume catalog serialize failed: {e}"),
        }
    }
}

/// Validate that a volume name contains only safe alphanumeric/dash/underscore/dot chars
/// and cannot perform path traversal.
pub fn validate_volume_name(name: &str) -> Result<()> {
    anyhow::ensure!(!name.is_empty(), "volume name cannot be empty");
    anyhow::ensure!(name.len() <= 64, "volume name too long (max 64 chars)");
    anyhow::ensure!(
        name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'),
        "volume name contains invalid characters: only [a-zA-Z0-9._-] permitted"
    );
    anyhow::ensure!(
        !name.contains("..") && !name.contains('/') && !name.contains('\\'),
        "volume name cannot contain directory traversal components"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_volume_name() {
        assert!(validate_volume_name("db-data").is_ok());
        assert!(validate_volume_name("my_volume.v1").is_ok());
        assert!(validate_volume_name("app123").is_ok());

        // Path traversal attempts
        assert!(validate_volume_name("../etc").is_err());
        assert!(validate_volume_name("foo/bar").is_err());
        assert!(validate_volume_name("foo\\bar").is_err());
        assert!(validate_volume_name("..").is_err());
        assert!(validate_volume_name("").is_err());
        assert!(validate_volume_name("a".repeat(65).as_str()).is_err());
    }
}
