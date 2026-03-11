use thiserror::Error;

/// Errors originating from the VeloceNet DNS or SOCKS5 layer.
#[derive(Debug, Error)]
pub enum NetError {
    #[error("DNS parse error: {0}")]
    DnsParse(String),

    #[error("hostname not registered: {0}")]
    NotRegistered(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
