use base64::Engine;
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::settings::Settings;

const API_BASE: &str = "https://api.cloudflare.com/client/v4";

pub struct CloudflareApi {
    client: reqwest::Client,
    api_token: String,
}

#[derive(Deserialize)]
struct ApiResponse<T> {
    success: bool,
    errors: Vec<ApiError>,
    result: Option<T>,
}

#[derive(Deserialize)]
struct ApiError {
    code: Option<i64>,
    message: String,
}

#[derive(Deserialize)]
struct Zone {
    name: String,
}

#[derive(Deserialize)]
struct Account {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct ZoneListItem {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct Tunnel {
    id: String,
    name: Option<String>,
    status: Option<String>,
}

/// Public tunnel info returned by `list_tunnels`.
#[derive(Debug, Clone)]
pub struct TunnelInfo {
    pub id: String,
    pub name: String,
    pub status: Option<String>,
}

#[derive(Deserialize)]
struct DnsRecord {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "type", default)]
    record_type: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

/// Public DNS record info returned by `list_dns_records`.
#[derive(Debug, Clone)]
pub struct DnsRecordInfo {
    pub id: String,
    pub name: String,
    pub record_type: String,
    pub content: String,
}

impl CloudflareApi {
    pub fn new(api_token: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_token: api_token.to_string(),
        }
    }

    fn api_error<T>(resp: &ApiResponse<T>) -> Error {
        if resp.errors.is_empty() {
            return Error::Tunnel("Cloudflare API: unknown error".into());
        }
        let msgs: Vec<String> = resp
            .errors
            .iter()
            .map(|e| match e.code {
                Some(code) => format!("[{}] {}", code, e.message),
                None => e.message.clone(),
            })
            .collect();
        Error::Tunnel(format!("Cloudflare API: {}", msgs.join("; ")))
    }

    /// Verify the API token is valid.
    pub async fn validate_token(&self) -> Result<bool> {
        let resp: ApiResponse<serde_json::Value> = self
            .client
            .get(format!("{}/user/tokens/verify", API_BASE))
            .bearer_auth(&self.api_token)
            .send()
            .await?
            .json()
            .await?;
        Ok(resp.success)
    }

    /// List all accounts accessible by this token.
    pub async fn list_accounts(&self) -> Result<Vec<(String, String)>> {
        let resp: ApiResponse<Vec<Account>> = self
            .client
            .get(format!("{}/accounts", API_BASE))
            .bearer_auth(&self.api_token)
            .send()
            .await?
            .json()
            .await?;

        if !resp.success {
            return Err(Self::api_error(&resp));
        }
        Ok(resp
            .result
            .unwrap_or_default()
            .into_iter()
            .map(|a| (a.id, a.name))
            .collect())
    }

    /// List all zones accessible by this token.
    pub async fn list_zones(&self) -> Result<Vec<(String, String)>> {
        let resp: ApiResponse<Vec<ZoneListItem>> = self
            .client
            .get(format!("{}/zones", API_BASE))
            .bearer_auth(&self.api_token)
            .send()
            .await?
            .json()
            .await?;

        if !resp.success {
            return Err(Self::api_error(&resp));
        }
        Ok(resp
            .result
            .unwrap_or_default()
            .into_iter()
            .map(|z| (z.id, z.name))
            .collect())
    }

    pub async fn get_zone_name(&self, zone_id: &str) -> Result<String> {
        let resp: ApiResponse<Zone> = self
            .client
            .get(format!("{}/zones/{}", API_BASE, zone_id))
            .bearer_auth(&self.api_token)
            .send()
            .await?
            .json()
            .await?;

        if !resp.success {
            return Err(Self::api_error(&resp));
        }
        resp.result
            .map(|z| z.name)
            .ok_or_else(|| Error::Tunnel("no zone data returned".into()))
    }

    /// Check if a tunnel exists and is not deleted.
    pub async fn tunnel_exists(&self, account_id: &str, tunnel_id: &str) -> Result<bool> {
        let http_resp = self
            .client
            .get(format!(
                "{}/accounts/{}/cfd_tunnel/{}",
                API_BASE, account_id, tunnel_id
            ))
            .bearer_auth(&self.api_token)
            .send()
            .await?;

        if !http_resp.status().is_success() {
            return Ok(false);
        }

        let resp: ApiResponse<Tunnel> = http_resp.json().await?;
        if let Some(t) = &resp.result {
            if let Some(status) = &t.status {
                return Ok(status != "deleted");
            }
        }
        Ok(resp.success)
    }

    pub async fn create_tunnel(
        &self,
        account_id: &str,
        name: &str,
        secret: &[u8; 32],
    ) -> Result<String> {
        let secret_b64 = base64::engine::general_purpose::STANDARD.encode(secret);

        let body = serde_json::json!({
            "name": name,
            "tunnel_secret": secret_b64,
            "config_src": "cloudflare",
        });

        let http_resp = self
            .client
            .post(format!(
                "{}/accounts/{}/cfd_tunnel",
                API_BASE, account_id
            ))
            .bearer_auth(&self.api_token)
            .json(&body)
            .send()
            .await?;

        let status = http_resp.status();
        let body_text = http_resp.text().await?;

        let resp: ApiResponse<Tunnel> = serde_json::from_str(&body_text).map_err(|_| {
            Error::Tunnel(format!(
                "Cloudflare API returned HTTP {} — {}",
                status,
                &body_text[..body_text.len().min(200)]
            ))
        })?;

        if !resp.success {
            return Err(Self::api_error(&resp));
        }
        resp.result
            .map(|t| t.id)
            .ok_or_else(|| Error::Tunnel("no tunnel ID returned".into()))
    }

    /// Fetch the current tunnel ingress config.
    async fn get_ingress(
        &self,
        account_id: &str,
        tunnel_id: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let resp: ApiResponse<serde_json::Value> = self
            .client
            .get(format!(
                "{}/accounts/{}/cfd_tunnel/{}/configurations",
                API_BASE, account_id, tunnel_id
            ))
            .bearer_auth(&self.api_token)
            .send()
            .await?
            .json()
            .await?;

        if !resp.success {
            return Err(Self::api_error(&resp));
        }

        // Extract ingress array from result.config.ingress
        let entries = resp
            .result
            .and_then(|r| r.get("config")?.get("ingress")?.as_array().cloned())
            .unwrap_or_default();

        Ok(entries)
    }

    /// Write the full ingress config to the tunnel.
    async fn put_ingress(
        &self,
        account_id: &str,
        tunnel_id: &str,
        ingress: &[serde_json::Value],
    ) -> Result<()> {
        let body = serde_json::json!({
            "config": {
                "ingress": ingress
            }
        });

        let resp: ApiResponse<serde_json::Value> = self
            .client
            .put(format!(
                "{}/accounts/{}/cfd_tunnel/{}/configurations",
                API_BASE, account_id, tunnel_id
            ))
            .bearer_auth(&self.api_token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if !resp.success {
            return Err(Self::api_error(&resp));
        }
        Ok(())
    }

    /// Add a hostname to the tunnel ingress, merging with existing entries.
    /// Replaces any existing entry for the same hostname.
    pub async fn add_ingress(
        &self,
        account_id: &str,
        tunnel_id: &str,
        hostname: &str,
        proxy_port: u16,
    ) -> Result<()> {
        let existing = self.get_ingress(account_id, tunnel_id).await?;

        let mut entries: Vec<serde_json::Value> = existing
            .into_iter()
            .filter(|e| {
                // Keep entries that aren't our hostname and aren't the catch-all
                let h = e.get("hostname").and_then(|v| v.as_str());
                h.is_some() && h != Some(hostname)
            })
            .collect();

        // Add our entry
        entries.push(serde_json::json!({
            "hostname": hostname,
            "service": format!("http://localhost:{}", proxy_port),
        }));

        // Always end with catch-all 404
        entries.push(serde_json::json!({
            "service": "http_status:404"
        }));

        self.put_ingress(account_id, tunnel_id, &entries).await
    }

    /// Remove a hostname from the tunnel ingress config.
    pub async fn remove_ingress(
        &self,
        account_id: &str,
        tunnel_id: &str,
        hostname: &str,
    ) -> Result<()> {
        let existing = self.get_ingress(account_id, tunnel_id).await?;

        let mut entries: Vec<serde_json::Value> = existing
            .into_iter()
            .filter(|e| {
                let h = e.get("hostname").and_then(|v| v.as_str());
                h.is_some() && h != Some(hostname)
            })
            .collect();

        // Always end with catch-all 404
        entries.push(serde_json::json!({
            "service": "http_status:404"
        }));

        self.put_ingress(account_id, tunnel_id, &entries).await
    }

    /// Check if any DNS records exist matching a given name (exact match).
    pub async fn dns_records_exist(&self, zone_id: &str, name: &str) -> Result<bool> {
        let resp: ApiResponse<Vec<DnsRecord>> = self
            .client
            .get(format!("{}/zones/{}/dns_records", API_BASE, zone_id))
            .bearer_auth(&self.api_token)
            .query(&[("name", name)])
            .send()
            .await?
            .json()
            .await?;

        if !resp.success {
            return Err(Self::api_error(&resp));
        }
        Ok(resp.result.map(|r| !r.is_empty()).unwrap_or(false))
    }

    pub async fn create_dns_record(
        &self,
        zone_id: &str,
        hostname: &str,
        tunnel_id: &str,
    ) -> Result<String> {
        let body = serde_json::json!({
            "type": "CNAME",
            "name": hostname,
            "content": format!("{}.cfargotunnel.com", tunnel_id),
            "proxied": true,
        });

        let resp: ApiResponse<DnsRecord> = self
            .client
            .post(format!("{}/zones/{}/dns_records", API_BASE, zone_id))
            .bearer_auth(&self.api_token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if !resp.success {
            return Err(Self::api_error(&resp));
        }
        resp.result
            .map(|r| r.id)
            .ok_or_else(|| Error::Tunnel("no DNS record ID returned".into()))
    }

    /// List all tunnels for an account.
    pub async fn list_tunnels(&self, account_id: &str) -> Result<Vec<TunnelInfo>> {
        let resp: ApiResponse<Vec<Tunnel>> = self
            .client
            .get(format!("{}/accounts/{}/cfd_tunnel", API_BASE, account_id))
            .bearer_auth(&self.api_token)
            .send()
            .await?
            .json()
            .await?;

        if !resp.success {
            return Err(Self::api_error(&resp));
        }
        Ok(resp
            .result
            .unwrap_or_default()
            .into_iter()
            .map(|t| TunnelInfo {
                id: t.id,
                name: t.name.unwrap_or_default(),
                status: t.status,
            })
            .collect())
    }

    /// List all DNS records for a zone.
    pub async fn list_dns_records(&self, zone_id: &str) -> Result<Vec<DnsRecordInfo>> {
        let resp: ApiResponse<Vec<DnsRecord>> = self
            .client
            .get(format!("{}/zones/{}/dns_records", API_BASE, zone_id))
            .bearer_auth(&self.api_token)
            .send()
            .await?
            .json()
            .await?;

        if !resp.success {
            return Err(Self::api_error(&resp));
        }
        Ok(resp
            .result
            .unwrap_or_default()
            .into_iter()
            .map(|r| DnsRecordInfo {
                id: r.id,
                name: r.name.unwrap_or_default(),
                record_type: r.record_type.unwrap_or_default(),
                content: r.content.unwrap_or_default(),
            })
            .collect())
    }

    /// Delete a DNS record. Used for teardown/purge.
    pub async fn delete_dns_record(&self, zone_id: &str, record_id: &str) -> Result<()> {
        let _ = self
            .client
            .delete(format!(
                "{}/zones/{}/dns_records/{}",
                API_BASE, zone_id, record_id
            ))
            .bearer_auth(&self.api_token)
            .send()
            .await?;
        Ok(())
    }

    /// Delete a tunnel. Used for teardown/purge.
    pub async fn delete_tunnel(&self, account_id: &str, tunnel_id: &str) -> Result<()> {
        let _ = self
            .client
            .delete(format!(
                "{}/accounts/{}/cfd_tunnel/{}",
                API_BASE, account_id, tunnel_id
            ))
            .bearer_auth(&self.api_token)
            .send()
            .await?;
        Ok(())
    }
}

fn generate_secret() -> Result<[u8; 32]> {
    let mut buf = [0u8; 32];
    #[cfg(unix)]
    {
        use std::io::Read;
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut buf))
            .map_err(|e| Error::Tunnel(format!("failed to generate secret: {}", e)))?;
    }
    #[cfg(not(unix))]
    {
        use std::time::SystemTime;
        let seed = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for (i, b) in buf.iter_mut().enumerate() {
            *b = ((seed >> (i % 16)) ^ (i as u128 * 0x9e3779b97f4a7c15)) as u8;
        }
    }
    Ok(buf)
}

/// Derive the cloudflared connector token from account ID, tunnel ID, and secret.
fn derive_token(account_id: &str, tunnel_id: &str, secret: &[u8; 32]) -> String {
    let secret_b64 = base64::engine::general_purpose::STANDARD.encode(secret);
    let token_json = serde_json::json!({
        "a": account_id,
        "t": tunnel_id,
        "s": secret_b64,
    });
    base64::engine::general_purpose::STANDARD.encode(token_json.to_string().as_bytes())
}

/// Generate a random subdomain like "amber-calm-river-dawn".
pub fn random_subdomain() -> String {
    const WORDS: &[&str] = &[
        "amber", "autumn", "azure", "bloom", "bold", "breeze", "bright", "calm",
        "cedar", "clear", "cloud", "coral", "creek", "crisp", "dawn", "deep",
        "delta", "dusk", "echo", "ember", "fern", "field", "flame", "flint",
        "flora", "frost", "gale", "gleam", "glow", "grace", "grove", "haze",
        "ivory", "jade", "keen", "lake", "leaf", "light", "lunar", "maple",
        "mist", "moss", "north", "oak", "opal", "palm", "pearl", "pine",
        "pond", "pulse", "rain", "reef", "ridge", "river", "sage", "sand",
        "shade", "silk", "silver", "sky", "snow", "solar", "south", "spark",
        "spring", "star", "steel", "stone", "storm", "sun", "swift", "teal",
        "tide", "trail", "vale", "wave", "west", "wild", "wind", "wood",
    ];

    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ (std::process::id() as u64) << 32;

    let mut parts = Vec::with_capacity(4);
    for _ in 0..4 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        parts.push(WORDS[((seed >> 33) as usize) % WORDS.len()]);
    }
    parts.join("-")
}

/// Resolve a `--host` argument into a full hostname.
///
/// Uses postfix format: `host-base.zone` (single subdomain level)
/// so Cloudflare's free Universal SSL cert (`*.zone`) covers it.
/// The base suffix ensures we never collide with real records (www, mail, etc).
pub fn resolve_hostname(host: &str, base: &str, zone: &str) -> String {
    // host-base.zone: myapp → myapp-sandbox.raiven.io
    format!("{}-{}.{}", host, base, zone)
}

/// Ensure the persistent tunnel and wildcard DNS exist.
///
/// On first run: creates tunnel + wildcard CNAME, saves state to settings.
/// On subsequent runs: verifies saved tunnel still exists, re-provisions if deleted.
pub async fn ensure_tunnel(settings: &mut Settings) -> Result<()> {
    let api = CloudflareApi::new(&settings.api_token);

    // If we have a saved tunnel, check it still exists
    if settings.has_tunnel() {
        match api
            .tunnel_exists(&settings.account_id, &settings.tunnel_id)
            .await
        {
            Ok(true) => return Ok(()),
            Ok(false) => {
                tracing::warn!("saved tunnel was deleted externally, re-provisioning...");
                settings.clear_tunnel();
            }
            Err(e) => {
                tracing::warn!("could not verify tunnel: {}, re-provisioning...", e);
                settings.clear_tunnel();
            }
        }
    }

    // Fetch zone name
    let zone_name = api.get_zone_name(&settings.zone_id).await?;

    // Generate secret and create tunnel
    let secret = generate_secret()?;
    let tunnel_name = format!("cfproxy-{}", settings.base_subdomain);
    let tunnel_id = api
        .create_tunnel(&settings.account_id, &tunnel_name, &secret)
        .await?;
    let token = derive_token(&settings.account_id, &tunnel_id, &secret);

    // Create wildcard DNS: *.zone → tunnel_id.cfargotunnel.com
    // Uses zone-level wildcard so hostnames are single-level (covered by free SSL)
    let wildcard_host = format!("*.{}", zone_name);
    let record_id = match api
        .create_dns_record(&settings.zone_id, &wildcard_host, &tunnel_id)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            // Cleanup tunnel on DNS failure
            let _ = api
                .delete_tunnel(&settings.account_id, &tunnel_id)
                .await;
            return Err(e);
        }
    };

    // Persist tunnel state
    settings.tunnel_id = tunnel_id;
    settings.tunnel_token_stored = token;
    settings.wildcard_record_id = record_id;
    settings.zone_name = zone_name;
    settings.save().map_err(|e| Error::Tunnel(format!("failed to save settings: {}", e)))?;

    Ok(())
}

/// Add this run's hostname to the tunnel ingress (merges with existing entries).
pub async fn update_ingress(
    settings: &Settings,
    hostname: &str,
    proxy_port: u16,
) -> Result<()> {
    let api = CloudflareApi::new(&settings.api_token);
    api.add_ingress(
        &settings.account_id,
        &settings.tunnel_id,
        hostname,
        proxy_port,
    )
    .await
}

/// Remove this run's hostname from the tunnel ingress on exit.
pub async fn clear_ingress(settings: &Settings, hostname: &str) -> Result<()> {
    let api = CloudflareApi::new(&settings.api_token);
    api.remove_ingress(&settings.account_id, &settings.tunnel_id, hostname)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_subdomain_format() {
        let sub = random_subdomain();
        let parts: Vec<&str> = sub.split('-').collect();
        assert_eq!(parts.len(), 4);
        assert!(parts.iter().all(|p| !p.is_empty()));
    }

    #[test]
    fn random_subdomain_varies() {
        // Two calls should (almost certainly) produce different results
        let a = random_subdomain();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let b = random_subdomain();
        // This could theoretically fail but the probability is ~1/80^4
        assert_ne!(a, b);
    }

    #[test]
    fn resolve_hostname_simple() {
        assert_eq!(
            resolve_hostname("myapp", "sandbox", "example.com"),
            "myapp-sandbox.example.com"
        );
    }

    #[test]
    fn resolve_hostname_random_words() {
        assert_eq!(
            resolve_hostname("flint-pulse-grace-swift", "sandbox", "example.com"),
            "flint-pulse-grace-swift-sandbox.example.com"
        );
    }

    #[test]
    fn resolve_hostname_short_name() {
        assert_eq!(
            resolve_hostname("api", "proxy", "example.com"),
            "api-proxy.example.com"
        );
    }

    #[test]
    fn derive_token_is_base64_json() {
        let secret = [0u8; 32];
        let token = derive_token("acc123", "tunnel-uuid", &secret);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&token)
            .expect("should decode base64");
        let json: serde_json::Value =
            serde_json::from_slice(&decoded).expect("should parse JSON");
        assert_eq!(json["a"], "acc123");
        assert_eq!(json["t"], "tunnel-uuid");
        assert!(json["s"].is_string());
    }

    #[test]
    fn generate_secret_produces_32_bytes() {
        let s = generate_secret().unwrap();
        assert_eq!(s.len(), 32);
    }
}
