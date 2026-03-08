use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

pub struct BinaryManager {
    cache_dir: PathBuf,
    auto_download: bool,
    override_path: Option<PathBuf>,
}

impl BinaryManager {
    pub fn new(cache_dir: PathBuf, auto_download: bool, override_path: Option<PathBuf>) -> Self {
        Self {
            cache_dir,
            auto_download,
            override_path,
        }
    }

    /// Return a path to a usable cloudflared binary.
    ///
    /// Resolution order:
    /// 1. Explicit override path (--cloudflared-path / CFPROXY_CLOUDFLARED_PATH)
    /// 2. Cached binary in cache_dir
    /// 3. System PATH lookup
    /// 4. Auto-download (if enabled)
    pub async fn ensure(&self) -> Result<PathBuf> {
        // 1. Explicit override
        if let Some(ref p) = self.override_path {
            return if p.exists() {
                eprintln!("  Using cloudflared at {} (explicit path)", p.display());
                Ok(p.clone())
            } else {
                Err(Error::BinaryNotFound { path: p.clone() })
            };
        }

        // 2. Cached binary
        let cached = self.cached_path();
        if cached.exists() {
            eprintln!("  Using cloudflared at {} (cached)", cached.display());
            return Ok(cached);
        }

        // 3. System PATH
        if let Some(p) = find_in_path() {
            eprintln!("  Using cloudflared at {} (system)", p.display());
            return Ok(p);
        }

        // 4. Auto-download
        if self.auto_download {
            eprintln!("  cloudflared not found, downloading...");
            std::fs::create_dir_all(&self.cache_dir)?;
            download(&cached).await?;
            eprintln!("  Downloaded cloudflared to {}", cached.display());
            return Ok(cached);
        }

        Err(Error::BinaryNotFound { path: cached })
    }

    fn cached_path(&self) -> PathBuf {
        let name = if cfg!(target_os = "windows") {
            "cloudflared.exe"
        } else {
            "cloudflared"
        };
        self.cache_dir.join(name)
    }
}

/// Delete the cached cloudflared binary and re-download the latest version.
pub async fn update(cache_dir: PathBuf) -> Result<()> {
    let dest = cache_dir.join("cloudflared");
    if dest.exists() {
        std::fs::remove_file(&dest)?;
        eprintln!("  Removed cached binary at {}", dest.display());
    }
    std::fs::create_dir_all(&cache_dir)?;
    eprintln!("  Downloading latest cloudflared...");
    download(&dest).await?;
    eprintln!("  Updated cloudflared at {}", dest.display());
    Ok(())
}

fn find_in_path() -> Option<PathBuf> {
    which("cloudflared")
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join(name);
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        })
    })
}

/// Download the cloudflared binary for the current platform.
async fn download(dest: &Path) -> Result<()> {
    let url = download_url()?;
    tracing::info!("fetching {}", url);

    let bytes = reqwest::get(&url)
        .await?
        .error_for_status()
        .map_err(|e| Error::Download(e.to_string()))?
        .bytes()
        .await?;

    if url.ends_with(".tgz") {
        extract_tgz(&bytes, dest)?;
    } else {
        std::fs::write(dest, &bytes)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
    }

    Ok(())
}

fn download_url() -> Result<String> {
    let base = "https://github.com/cloudflare/cloudflared/releases/latest/download";

    let url = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => format!("{}/cloudflared-darwin-arm64.tgz", base),
        ("macos", "x86_64") => format!("{}/cloudflared-darwin-amd64.tgz", base),
        ("linux", "x86_64") => format!("{}/cloudflared-linux-amd64", base),
        ("linux", "aarch64") => format!("{}/cloudflared-linux-arm64", base),
        (os, arch) => {
            return Err(Error::UnsupportedPlatform {
                os: os.into(),
                arch: arch.into(),
            })
        }
    };
    Ok(url)
}

fn extract_tgz(data: &[u8], dest: &Path) -> Result<()> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let decoder = GzDecoder::new(data);
    let mut archive = Archive::new(decoder);

    let parent = dest.parent().unwrap_or(Path::new("."));

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        if path.file_name().and_then(|n| n.to_str()) == Some("cloudflared") {
            entry.unpack(dest)?;
            return Ok(());
        }
        // If only one file, extract it regardless of name
        entry.unpack(parent.join(path.file_name().unwrap_or(path.as_ref().as_ref())))?;
    }

    if dest.exists() {
        Ok(())
    } else {
        Err(Error::Download(
            "cloudflared binary not found in archive".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_url_returns_valid_url() {
        // This test will pass on supported platforms
        let result = download_url();
        match std::env::consts::OS {
            "macos" | "linux" => {
                let url = result.unwrap();
                assert!(url.starts_with("https://"));
                assert!(url.contains("cloudflared"));
            }
            _ => {
                assert!(result.is_err());
            }
        }
    }

    #[test]
    fn which_finds_common_binary() {
        // "sh" should exist on any unix system
        if cfg!(unix) {
            assert!(which("sh").is_some());
        }
    }

    #[test]
    fn which_returns_none_for_missing() {
        assert!(which("nonexistent_binary_xyz_12345").is_none());
    }

    #[test]
    fn cached_path_has_correct_name() {
        let mgr = BinaryManager::new(PathBuf::from("/tmp/cache"), true, None);
        let path = mgr.cached_path();
        assert_eq!(path, PathBuf::from("/tmp/cache/cloudflared"));
    }

    #[tokio::test]
    async fn ensure_with_override_missing_file() {
        let mgr = BinaryManager::new(
            PathBuf::from("/tmp"),
            true,
            Some(PathBuf::from("/nonexistent/cloudflared")),
        );
        let result = mgr.ensure().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn ensure_no_download_no_binary() {
        let mgr = BinaryManager::new(
            PathBuf::from("/tmp/cfproxy-test-nonexistent"),
            false, // auto_download disabled
            None,
        );
        // If cloudflared is not in PATH, this should fail.
        // If it IS in PATH, it will succeed (which is fine).
        let result = mgr.ensure().await;
        if find_in_path().is_none() {
            assert!(result.is_err());
        }
    }
}
