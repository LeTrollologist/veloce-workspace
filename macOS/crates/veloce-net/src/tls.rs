/*!
# TLS Engine for VeloceNet Ingress (v3.2)

Provides TLS termination, dynamic SNI certificate resolution, and zero-configuration
ephemeral self-signed certificates with SANs for `*.vln` domains.
*/

use anyhow::{bail, Context, Result};
use parking_lot::RwLock;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::HashMap;
use std::sync::Arc;
use tokio_rustls::rustls::server::{ClientHello, ResolvesServerCert};
use tokio_rustls::rustls::sign::CertifiedKey;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, info};

/// Dynamic SNI Certificate Resolver supporting per-host certificates and a default wildcard fallback.
#[derive(Debug)]
pub struct SniCertResolver {
    default_key: Arc<CertifiedKey>,
    host_keys: RwLock<HashMap<String, Arc<CertifiedKey>>>,
}

impl SniCertResolver {
    pub fn new(default_key: Arc<CertifiedKey>) -> Self {
        Self {
            default_key,
            host_keys: RwLock::new(HashMap::new()),
        }
    }

    pub fn insert_host_cert(&self, host: &str, key: Arc<CertifiedKey>) {
        self.host_keys.write().insert(host.to_lowercase(), key);
    }

    pub fn remove_host_cert(&self, host: &str) {
        self.host_keys.write().remove(&host.to_lowercase());
    }
}

impl ResolvesServerCert for SniCertResolver {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        if let Some(server_name) = client_hello.server_name() {
            let map = self.host_keys.read();
            if let Some(cert) = map.get(&server_name.to_lowercase()) {
                debug!(sni = %server_name, "tls: matched host-specific certificate");
                return Some(Arc::clone(cert));
            }
        }

        debug!("tls: falling back to default wildcard certificate");
        Some(Arc::clone(&self.default_key))
    }
}

pub struct TlsManager {
    acceptor: Arc<TlsAcceptor>,
    resolver: Arc<SniCertResolver>,
}

impl TlsManager {
    /// Create a TlsManager with an automatically generated ephemeral self-signed CA & certificate for `*.vln`.
    pub fn new_self_signed() -> Result<Self> {
        let (cert_der, key_der) = generate_self_signed_vln_cert()?;
        let signing_key = tokio_rustls::rustls::crypto::ring::sign::any_supported_type(&key_der)
            .map_err(|e| anyhow::anyhow!("failed to load signing key: {e:?}"))?;

        let certified_key = Arc::new(CertifiedKey::new(vec![cert_der], signing_key));
        let resolver = Arc::new(SniCertResolver::new(Arc::clone(&certified_key)));

        let server_config = ServerConfig::builder_with_provider(Arc::new(tokio_rustls::rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .context("safe protocol versions")?
            .with_no_client_auth()
            .with_cert_resolver(Arc::clone(&resolver) as Arc<dyn ResolvesServerCert>);

        info!("tls: Ingress TLS engine initialized with ephemeral *.vln self-signed certificate");

        Ok(Self {
            acceptor: Arc::new(TlsAcceptor::from(Arc::new(server_config))),
            resolver,
        })
    }

    /// Register a custom certificate and key in PEM format for a specific hostname.
    pub fn add_custom_cert(&self, host: &str, cert_pem: &str, key_pem: &str) -> Result<()> {
        let certs = parse_pem_certs(cert_pem)?;
        let key_der = parse_pem_key(key_pem)?;
        let signing_key = tokio_rustls::rustls::crypto::ring::sign::any_supported_type(&key_der)
            .map_err(|e| anyhow::anyhow!("failed to load custom signing key: {e:?}"))?;

        let certified_key = Arc::new(CertifiedKey::new(certs, signing_key));
        self.resolver.insert_host_cert(host, certified_key);
        info!(host = %host, "tls: custom certificate registered");
        Ok(())
    }

    /// Remove a custom certificate for a specific hostname.
    pub fn remove_custom_cert(&self, host: &str) {
        self.resolver.remove_host_cert(host);
    }

    pub fn acceptor(&self) -> Arc<TlsAcceptor> {
        Arc::clone(&self.acceptor)
    }
}

/// Generates an ephemeral self-signed ECDSA certificate covering `*.vln`, `localhost`, and loopback IPs.
pub fn generate_self_signed_vln_cert() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let keypair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .context("generate ECDSA keypair")?;

    let mut params = CertificateParams::new(vec![
        "*.vln".to_string(),
        "vln".to_string(),
        "localhost".to_string(),
    ])
    .context("create certificate params")?;

    params.subject_alt_names.push(SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)));
    params.subject_alt_names.push(SanType::IpAddress(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)));

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "*.vln");
    dn.push(DnType::OrganizationName, "VeloceNetwork Self-Signed Ingress");
    params.distinguished_name = dn;

    let cert = params.self_signed(&keypair).context("self-sign certificate")?;
    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(keypair.serialize_der().into());

    Ok((cert_der, key_der))
}

fn parse_pem_certs(pem: &str) -> Result<Vec<CertificateDer<'static>>> {
    let mut certs = Vec::new();
    for item in rustls_pemfile::certs(&mut pem.as_bytes()) {
        let cert = item.context("parse PEM certificate")?;
        certs.push(cert);
    }
    if certs.is_empty() {
        bail!("no certificates found in PEM data");
    }
    Ok(certs)
}

fn parse_pem_key(pem: &str) -> Result<PrivateKeyDer<'static>> {
    for item in rustls_pemfile::read_all(&mut pem.as_bytes()) {
        match item.context("read PEM key item")? {
            rustls_pemfile::Item::Pkcs1Key(k) => return Ok(PrivateKeyDer::Pkcs1(k)),
            rustls_pemfile::Item::Pkcs8Key(k) => return Ok(PrivateKeyDer::Pkcs8(k)),
            rustls_pemfile::Item::Sec1Key(k) => return Ok(PrivateKeyDer::Sec1(k)),
            _ => continue,
        }
    }
    bail!("no valid private key found in PEM data")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_self_signed_cert() {
        let (cert, key) = generate_self_signed_vln_cert().unwrap();
        assert!(!cert.is_empty());
        match key {
            PrivateKeyDer::Pkcs8(k) => assert!(!k.secret_pkcs8_der().is_empty()),
            _ => panic!("expected pkcs8 key"),
        }
    }

    #[test]
    fn test_tls_manager_initialization() {
        let mgr = TlsManager::new_self_signed().unwrap();
        let _acceptor = mgr.acceptor();
    }
}
