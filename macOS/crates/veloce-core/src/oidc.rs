/*!
OpenID Connect (OIDC) & Corporate Identity Engine.

Manages Enterprise Single Sign-On (SSO) authentication state, OIDC token claims parsing,
active user sessions, and persistence.
*/

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use veloce_ipc::message::{OidcAuthInfoMsg, OidcSessionMsg};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcSession {
    pub issuer_url: String,
    pub client_id: String,
    pub subject: String,
    pub email: String,
    pub name: Option<String>,
    pub groups: Vec<String>,
    pub id_token: String,
    pub expires_at: u64,
}

impl From<OidcSessionMsg> for OidcSession {
    fn from(m: OidcSessionMsg) -> Self {
        Self {
            issuer_url: m.issuer_url,
            client_id: m.client_id,
            subject: m.subject,
            email: m.email,
            name: m.name,
            groups: m.groups,
            id_token: m.id_token,
            expires_at: m.expires_at,
        }
    }
}

impl From<OidcSession> for OidcSessionMsg {
    fn from(s: OidcSession) -> Self {
        Self {
            issuer_url: s.issuer_url,
            client_id: s.client_id,
            subject: s.subject,
            email: s.email,
            name: s.name,
            groups: s.groups,
            id_token: s.id_token,
            expires_at: s.expires_at,
        }
    }
}

/// Parsed JWT claims from an OpenID Connect ID Token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcJwtClaims {
    #[serde(default)]
    pub iss: Option<String>,
    #[serde(default)]
    pub sub: Option<String>,
    #[serde(default)]
    pub aud: Option<serde_json::Value>,
    #[serde(default)]
    pub exp: Option<u64>,
    #[serde(default)]
    pub iat: Option<u64>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub preferred_username: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub groups: Option<Vec<String>>,
    #[serde(default)]
    pub roles: Option<Vec<String>>,
}

pub struct OidcEngine {
    session: Arc<RwLock<Option<OidcSession>>>,
    session_file: PathBuf,
}

impl OidcEngine {
    pub fn new(data_dir: &Path) -> Self {
        let session_file = data_dir.join("sso-session.json");
        let initial = Self::load_session_file(&session_file);
        Self {
            session: Arc::new(RwLock::new(initial)),
            session_file,
        }
    }

    fn load_session_file(path: &Path) -> Option<OidcSession> {
        if !path.exists() {
            return None;
        }
        let data = std::fs::read_to_string(path).ok()?;
        let session: OidcSession = serde_json::from_str(&data).ok()?;

        // Check if expired
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if session.expires_at > 0 && now > session.expires_at {
            tracing::info!("Stored SSO session for {} has expired", session.email);
            return None;
        }

        tracing::info!("Restored active SSO session for {}", session.email);
        Some(session)
    }

    pub fn set_session(&self, session: OidcSession) -> Result<()> {
        let json = serde_json::to_string_pretty(&session)
            .context("serialize SSO session")?;
        if let Some(parent) = self.session_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&self.session_file, json)
            .with_context(|| format!("save SSO session to {}", self.session_file.display()))?;

        tracing::info!(
            "SSO authenticated: {} (groups: {:?})",
            session.email,
            session.groups
        );
        *self.session.write() = Some(session);
        Ok(())
    }

    pub fn clear_session(&self) -> Result<()> {
        if self.session_file.exists() {
            let _ = std::fs::remove_file(&self.session_file);
        }
        *self.session.write() = None;
        tracing::info!("SSO session cleared (logged out)");
        Ok(())
    }

    pub fn get_session(&self) -> Option<OidcSession> {
        self.session.read().clone()
    }

    pub fn get_auth_info(&self) -> OidcAuthInfoMsg {
        if let Some(s) = self.session.read().as_ref() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let is_valid = s.expires_at == 0 || now < s.expires_at;

            OidcAuthInfoMsg {
                is_authenticated: is_valid,
                issuer_url: Some(s.issuer_url.clone()),
                subject: Some(s.subject.clone()),
                email: Some(s.email.clone()),
                name: s.name.clone(),
                groups: s.groups.clone(),
                expires_at: Some(s.expires_at),
            }
        } else {
            OidcAuthInfoMsg {
                is_authenticated: false,
                issuer_url: None,
                subject: None,
                email: None,
                name: None,
                groups: vec![],
                expires_at: None,
            }
        }
    }

    /// Extract and decode claims from a raw JWT ID Token string without external crypto verifiers.
    pub fn parse_jwt_claims(token: &str) -> Result<OidcJwtClaims> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() < 2 {
            anyhow::bail!("invalid JWT token format (expected at least 2 dot-separated segments)");
        }

        let payload_b64 = parts[1];
        let decoded = URL_SAFE_NO_PAD
            .decode(payload_b64)
            .or_else(|_| base64::engine::general_purpose::STANDARD.decode(payload_b64))
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(payload_b64))
            .context("base64 decode JWT payload")?;

        let claims: OidcJwtClaims = serde_json::from_slice(&decoded)
            .context("parse JWT claims JSON payload")?;
        Ok(claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_claims_parsing() {
        // Sample payload: {"sub":"user-123","email":"alice@company.com","name":"Alice Dev","groups":["Engineering","DevOps"],"exp":1999999999}
        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ1c2VyLTEyMyIsImVtYWlsIjoiYWxpY2VAY29tcGFueS5jb20iLCJuYW1lIjoiQWxpY2UgRGV2IiwiZ3JvdXBzIjpbIkVuZ2luZWVyaW5nIiwiRGV2T3BzIl0sImV4cCI6MTk5OTk5OTk5OX0.dummy_sig";
        let claims = OidcEngine::parse_jwt_claims(jwt).expect("parse claims");
        assert_eq!(claims.sub, Some("user-123".into()));
        assert_eq!(claims.email, Some("alice@company.com".into()));
        assert_eq!(claims.groups, Some(vec!["Engineering".into(), "DevOps".into()]));
    }

    #[test]
    fn test_oidc_engine_session_lifecycle() {
        let dir = std::env::temp_dir().join(format!("veloce-test-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        let engine = OidcEngine::new(&dir);
        assert!(!engine.get_auth_info().is_authenticated);

        let session = OidcSession {
            issuer_url: "https://login.microsoftonline.com/test/v2.0".into(),
            client_id: "veloce-app".into(),
            subject: "user-456".into(),
            email: "bob@company.com".into(),
            name: Some("Bob".into()),
            groups: vec!["DevOps".into()],
            id_token: "dummy.jwt.token".into(),
            expires_at: 2999999999,
        };

        engine.set_session(session.clone()).expect("set session");
        let info = engine.get_auth_info();
        assert!(info.is_authenticated);
        assert_eq!(info.email, Some("bob@company.com".into()));
        assert_eq!(info.groups, vec!["DevOps".to_string()]);

        // Verify restoration from disk
        let engine2 = OidcEngine::new(&dir);
        assert!(engine2.get_auth_info().is_authenticated);

        // Clear session
        engine.clear_session().expect("clear");
        assert!(!engine.get_auth_info().is_authenticated);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
