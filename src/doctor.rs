use std::path::PathBuf;

use crate::cloudflare::CloudflareApi;
use crate::cloudflared::BinaryManager;
use crate::error::Result;
use crate::settings::Settings;

// ---------------------------------------------------------------------------
// Diagnostic result types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticResult {
    Pass,
    Fail,
    Skip,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub label: String,
    pub result: DiagnosticResult,
    pub detail: String,
}

impl Check {
    pub fn pass(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            result: DiagnosticResult::Pass,
            detail: detail.into(),
        }
    }

    pub fn fail(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            result: DiagnosticResult::Fail,
            detail: detail.into(),
        }
    }

    pub fn skip(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            result: DiagnosticResult::Skip,
            detail: detail.into(),
        }
    }

    /// Print the check result with ANSI colors.
    pub fn print(&self) {
        let (icon, color) = match self.result {
            DiagnosticResult::Pass => ("\u{2713}", "\x1b[32m"), // green checkmark
            DiagnosticResult::Fail => ("\u{2717}", "\x1b[31m"), // red x
            DiagnosticResult::Skip => ("-", "\x1b[90m"),        // gray dash
        };
        let reset = "\x1b[0m";
        if self.detail.is_empty() {
            eprintln!("  {}{}{} {}", color, icon, reset, self.label);
        } else {
            eprintln!("  {}{}{} {} ({})", color, icon, reset, self.label, self.detail);
        }
    }
}

// ---------------------------------------------------------------------------
// Individual check functions
// ---------------------------------------------------------------------------

/// Check if the settings file exists and is valid JSON.
pub fn check_settings_file() -> Check {
    let path = settings_path();
    let display_path = path.display().to_string();

    match std::fs::read_to_string(&path) {
        Ok(contents) => match serde_json::from_str::<serde_json::Value>(&contents) {
            Ok(_) => Check::pass("Settings file found", display_path),
            Err(e) => Check::fail("Settings file invalid JSON", format!("{}: {}", display_path, e)),
        },
        Err(_) => Check::fail("Settings file not found", display_path),
    }
}

/// Check if cloudflared binary can be found and get its version.
pub async fn check_cloudflared_binary(
    cache_dir: PathBuf,
    override_path: Option<PathBuf>,
) -> Check {
    // Use BinaryManager with auto_download disabled to just locate existing binary
    let manager = BinaryManager::new(cache_dir, false, override_path);
    match manager.ensure().await {
        Ok(path) => {
            // Try to get version
            let version = get_cloudflared_version(&path).await;
            let detail = match version {
                Some(v) => format!("{}, version {}", path.display(), v),
                None => format!("{}", path.display()),
            };
            Check::pass("cloudflared binary found", detail)
        }
        Err(_) => Check::fail("cloudflared binary not found", "not in PATH or cache"),
    }
}

/// Get the version string from cloudflared.
async fn get_cloudflared_version(binary: &PathBuf) -> Option<String> {
    let output = tokio::process::Command::new(binary)
        .arg("--version")
        .output()
        .await
        .ok()?;

    let text = String::from_utf8_lossy(&output.stdout);
    // cloudflared version output is typically: "cloudflared version 2024.1.5 (built ...)"
    // or just the version on stdout. Also check stderr.
    let combined = if text.trim().is_empty() {
        String::from_utf8_lossy(&output.stderr).to_string()
    } else {
        text.to_string()
    };

    // Extract version number
    for word in combined.split_whitespace() {
        // Look for a word that looks like a version (starts with digit, contains dots)
        if word.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
            && word.contains('.')
        {
            return Some(word.to_string());
        }
    }

    // Fallback: return trimmed first line
    combined.lines().next().map(|l| l.trim().to_string())
}

/// Check network connectivity to Cloudflare API.
pub async fn check_network() -> Check {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(e) => return Check::fail("Network connectivity", format!("failed to create client: {}", e)),
    };

    match client.get("https://api.cloudflare.com/client/v4/").send().await {
        Ok(resp) => {
            if resp.status().is_success() || resp.status().is_client_error() {
                // 4xx is expected without auth — means we reached the server
                Check::pass("Network connectivity", "api.cloudflare.com reachable")
            } else {
                Check::fail(
                    "Network connectivity",
                    format!("api.cloudflare.com returned HTTP {}", resp.status()),
                )
            }
        }
        Err(e) => Check::fail("Network connectivity", format!("api.cloudflare.com unreachable: {}", e)),
    }
}

/// Validate the API token if configured.
pub async fn check_api_token(settings: &Settings) -> Check {
    if settings.api_token.trim().is_empty() {
        return Check::skip("API token", "not configured, skipped");
    }

    let api = CloudflareApi::new(&settings.api_token);
    match api.validate_token().await {
        Ok(true) => Check::pass("API token", "valid"),
        Ok(false) => Check::fail("API token", "invalid or expired"),
        Err(e) => Check::fail("API token", format!("validation failed: {}", e)),
    }
}

/// Check if the configured tunnel still exists.
pub async fn check_tunnel(settings: &Settings) -> Check {
    if !settings.has_tunnel() {
        return Check::skip("Tunnel status", "not configured, skipped");
    }

    if settings.api_token.trim().is_empty() {
        return Check::skip("Tunnel status", "no API token, skipped");
    }

    let api = CloudflareApi::new(&settings.api_token);
    match api
        .tunnel_exists(&settings.account_id, &settings.tunnel_id)
        .await
    {
        Ok(true) => Check::pass(
            "Tunnel status",
            format!("exists ({}...)", &settings.tunnel_id[..8.min(settings.tunnel_id.len())]),
        ),
        Ok(false) => Check::fail("Tunnel status", "tunnel not found or deleted"),
        Err(e) => Check::fail("Tunnel status", format!("check failed: {}", e)),
    }
}

/// Check if the wildcard DNS record exists.
pub async fn check_dns(settings: &Settings) -> Check {
    if settings.zone_id.trim().is_empty() || settings.zone_name.trim().is_empty() {
        return Check::skip("DNS records", "not configured, skipped");
    }

    if settings.api_token.trim().is_empty() {
        return Check::skip("DNS records", "no API token, skipped");
    }

    let api = CloudflareApi::new(&settings.api_token);
    let wildcard = format!("*.{}", settings.zone_name);
    match api.dns_records_exist(&settings.zone_id, &wildcard).await {
        Ok(true) => Check::pass("DNS records", format!("{} exists", wildcard)),
        Ok(false) => Check::fail("DNS records", format!("{} not found", wildcard)),
        Err(e) => Check::fail("DNS records", format!("check failed: {}", e)),
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub async fn run_doctor(
    cache_dir: Option<PathBuf>,
    override_path: Option<PathBuf>,
) -> Result<()> {
    eprintln!();
    eprintln!("  \x1b[1;36mcfproxy doctor\x1b[0m");
    eprintln!("  \x1b[90m──────────────\x1b[0m");
    eprintln!();

    let resolved_cache_dir = cache_dir.unwrap_or_else(default_cache_dir);

    // 1. Settings file
    let settings_check = check_settings_file();
    settings_check.print();

    // Load settings for subsequent checks
    let settings = Settings::load();

    // 2. cloudflared binary
    let binary_check = check_cloudflared_binary(resolved_cache_dir, override_path).await;
    binary_check.print();

    // 3. Network connectivity
    let network_check = check_network().await;
    network_check.print();

    // 4. API token
    let token_check = check_api_token(&settings).await;
    token_check.print();

    // 5. Tunnel status
    let tunnel_check = check_tunnel(&settings).await;
    tunnel_check.print();

    // 6. DNS records
    let dns_check = check_dns(&settings).await;
    dns_check.print();

    eprintln!();

    Ok(())
}

fn settings_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("cfproxy")
        .join("settings.json")
}

fn default_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("cfproxy")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_pass_construction() {
        let c = Check::pass("Test label", "some detail");
        assert_eq!(c.result, DiagnosticResult::Pass);
        assert_eq!(c.label, "Test label");
        assert_eq!(c.detail, "some detail");
    }

    #[test]
    fn check_fail_construction() {
        let c = Check::fail("Test label", "error detail");
        assert_eq!(c.result, DiagnosticResult::Fail);
        assert_eq!(c.label, "Test label");
        assert_eq!(c.detail, "error detail");
    }

    #[test]
    fn check_skip_construction() {
        let c = Check::skip("Test label", "not configured, skipped");
        assert_eq!(c.result, DiagnosticResult::Skip);
        assert_eq!(c.label, "Test label");
        assert_eq!(c.detail, "not configured, skipped");
    }

    #[test]
    fn settings_file_check_missing() {
        // The settings file may or may not exist on the test machine,
        // but we can verify the function doesn't panic
        let check = check_settings_file();
        assert!(
            check.result == DiagnosticResult::Pass || check.result == DiagnosticResult::Fail
        );
        assert!(check.label.contains("Settings file"));
    }

    #[test]
    fn api_token_skip_when_empty() {
        let settings = Settings::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let check = rt.block_on(check_api_token(&settings));
        assert_eq!(check.result, DiagnosticResult::Skip);
    }

    #[test]
    fn tunnel_skip_when_not_configured() {
        let settings = Settings::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let check = rt.block_on(check_tunnel(&settings));
        assert_eq!(check.result, DiagnosticResult::Skip);
    }

    #[test]
    fn dns_skip_when_not_configured() {
        let settings = Settings::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let check = rt.block_on(check_dns(&settings));
        assert_eq!(check.result, DiagnosticResult::Skip);
    }

    #[tokio::test]
    async fn binary_check_with_nonexistent_cache() {
        let check = check_cloudflared_binary(
            PathBuf::from("/tmp/cfproxy-doctor-test-nonexistent"),
            None,
        )
        .await;
        // May pass if cloudflared is in PATH, or fail if not
        assert!(
            check.result == DiagnosticResult::Pass || check.result == DiagnosticResult::Fail
        );
        assert!(check.label.contains("cloudflared"));
    }

    #[test]
    fn check_print_does_not_panic() {
        // Verify print doesn't panic for any result type
        Check::pass("test", "detail").print();
        Check::fail("test", "detail").print();
        Check::skip("test", "detail").print();
        Check::pass("test", "").print();
    }
}
