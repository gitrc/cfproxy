use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cloudflared binary not found at {path}")]
    BinaryNotFound { path: PathBuf },

    #[error("failed to download cloudflared: {0}")]
    Download(String),

    #[error("unsupported platform: {os}/{arch}")]
    UnsupportedPlatform { os: String, arch: String },

    #[error("tunnel process failed: {0}")]
    Tunnel(String),

    #[error("metrics fetch failed: {0}")]
    Metrics(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Http(#[from] reqwest::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
