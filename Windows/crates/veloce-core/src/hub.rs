/*!
# Veloce Hub — Catalog & Application Registry Engine (v3.4)

Provides userspace application registration, catalog searching,
template management, and 1-click deployment resolution.
*/

use anyhow::{Context, Result};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info, warn};
use veloce_ipc::message::HubAppMsg;

pub struct HubCatalogEngine {
    catalog_path: PathBuf,
    apps: RwLock<HashMap<String, HubAppMsg>>,
}

impl HubCatalogEngine {
    pub fn new(data_dir: &Path) -> Arc<Self> {
        let catalog_path = data_dir.join("hub-catalog.json");
        let mut apps = HashMap::new();

        if catalog_path.exists() {
            match std::fs::read_to_string(&catalog_path) {
                Ok(content) => match serde_json::from_str::<Vec<HubAppMsg>>(&content) {
                    Ok(list) => {
                        for app in list {
                            apps.insert(app.name.clone(), app);
                        }
                        info!("Loaded {} application(s) from Hub catalog", apps.len());
                    }
                    Err(e) => warn!("Failed to parse Hub catalog JSON: {e}"),
                },
                Err(e) => warn!("Failed to read Hub catalog file: {e}"),
            }
        }

        // If catalog is empty, populate curated starter templates
        if apps.is_empty() {
            let starters = default_starters();
            for s in starters {
                apps.insert(s.name.clone(), s);
            }
        }

        let engine = Arc::new(Self {
            catalog_path,
            apps: RwLock::new(apps),
        });

        // Persist default catalog if file doesn't exist
        let _ = engine.save();
        engine
    }

    /// Register or update an application entry in the Hub catalog.
    pub fn publish(&self, app: HubAppMsg) -> Result<()> {
        let name = app.name.clone();
        {
            let mut map = self.apps.write();
            map.insert(name.clone(), app);
        }
        self.save().with_context(|| format!("save Hub catalog after publishing '{name}'"))?;
        info!("Published application '{name}' to Veloce Hub");
        Ok(())
    }

    /// Retrieve an application entry by name.
    pub fn get(&self, name: &str) -> Option<HubAppMsg> {
        self.apps.read().get(name).cloned()
    }

    /// Remove an application from the catalog.
    pub fn remove(&self, name: &str) -> bool {
        let removed = {
            let mut map = self.apps.write();
            map.remove(name).is_some()
        };
        if removed {
            let _ = self.save();
            info!("Removed application '{name}' from Veloce Hub");
        }
        removed
    }

    /// List all catalog applications.
    pub fn list(&self) -> Vec<HubAppMsg> {
        let map = self.apps.read();
        let mut list: Vec<HubAppMsg> = map.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Search catalog applications by name, description, or category keyword.
    pub fn search(&self, query: &str) -> Vec<HubAppMsg> {
        let q = query.to_lowercase();
        let map = self.apps.read();
        let mut results = Vec::new();
        for app in map.values() {
            if app.name.to_lowercase().contains(&q)
                || app.description.to_lowercase().contains(&q)
                || app.category.to_lowercase().contains(&q)
                || app.author.to_lowercase().contains(&q)
            {
                results.push(app.clone());
            }
        }
        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }

    fn save(&self) -> Result<()> {
        let list: Vec<HubAppMsg> = self.apps.read().values().cloned().collect();
        let json = serde_json::to_string_pretty(&list)?;
        if let Some(parent) = self.catalog_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.catalog_path, json)?;
        Ok(())
    }
}

fn default_starters() -> Vec<HubAppMsg> {
    vec![
        HubAppMsg {
            name: "web-starter".into(),
            version: "1.0.0".into(),
            description: "High-performance HTTP static website server with auto-HTTPS".into(),
            category: "Web".into(),
            author: "VeloceSolutions".into(),
            executable: "veloce-run".into(),
            args: vec!["--help".into()],
            env: vec![("PORT".into(), "8000".into())],
            port: Some(8000),
            hostname: Some("web.vln".into()),
            cpu: Some(25),
            mem: Some(256),
            replicas: 1,
            auto_restart: true,
            tls: true,
        },
        HubAppMsg {
            name: "api-gateway".into(),
            version: "1.1.0".into(),
            description: "Layer-7 microservice API gateway router with load balancing".into(),
            category: "API".into(),
            author: "VeloceSolutions".into(),
            executable: "veloce-run".into(),
            args: vec!["ingress".into(), "list".into()],
            env: vec![("VLN_API_ENV".into(), "production".into())],
            port: Some(4000),
            hostname: Some("api.vln".into()),
            cpu: Some(50),
            mem: Some(512),
            replicas: 2,
            auto_restart: true,
            tls: true,
        },
        HubAppMsg {
            name: "redis-cache".into(),
            version: "7.2.0".into(),
            description: "Distributed in-memory key-value cache and pub/sub message broker".into(),
            category: "Database".into(),
            author: "VeloceSolutions".into(),
            executable: "redis-server".into(),
            args: vec![],
            env: vec![],
            port: Some(6379),
            hostname: Some("cache.vln".into()),
            cpu: Some(50),
            mem: Some(1024),
            replicas: 1,
            auto_restart: true,
            tls: false,
        },
        HubAppMsg {
            name: "echo-tester".into(),
            version: "1.0.0".into(),
            description: "Fast network round-trip diagnostic and echo service".into(),
            category: "Tools".into(),
            author: "VeloceSolutions".into(),
            executable: "veloce-run".into(),
            args: vec!["version".into()],
            env: vec![],
            port: Some(9999),
            hostname: Some("echo.vln".into()),
            cpu: Some(10),
            mem: Some(64),
            replicas: 1,
            auto_restart: false,
            tls: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hub_catalog_engine() {
        let temp_dir = std::env::temp_dir().join(format!("vln_test_hub_{}", uuid::Uuid::new_v4()));
        let engine = HubCatalogEngine::new(&temp_dir);

        let list = engine.list();
        assert!(list.len() >= 4);

        let search_res = engine.search("database");
        assert_eq!(search_res.len(), 1);
        assert_eq!(search_res[0].name, "redis-cache");

        let custom_app = HubAppMsg {
            name: "my-custom-service".into(),
            version: "0.1.0".into(),
            description: "Custom user service".into(),
            category: "Custom".into(),
            author: "Tester".into(),
            executable: "test-binary".into(),
            args: vec![],
            env: vec![],
            port: Some(5000),
            hostname: Some("custom.vln".into()),
            cpu: None,
            mem: None,
            replicas: 1,
            auto_restart: false,
            tls: false,
        };

        engine.publish(custom_app).unwrap();
        assert!(engine.get("my-custom-service").is_some());
        assert!(engine.remove("my-custom-service"));
        assert!(engine.get("my-custom-service").is_none());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
