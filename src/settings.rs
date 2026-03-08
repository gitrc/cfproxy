use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Legacy: manual tunnel token (deprecated, kept for backward compat).
    #[serde(default)]
    pub tunnel_token: String,

    /// Cloudflare API token with Tunnel:Edit and DNS:Edit permissions.
    #[serde(default)]
    pub api_token: String,

    /// Cloudflare account ID (from dashboard overview).
    #[serde(default)]
    pub account_id: String,

    /// Cloudflare zone ID for the domain to use (from domain overview).
    #[serde(default)]
    pub zone_id: String,

    /// Base subdomain namespace (e.g. "tunnel" → *.tunnel.example.com).
    #[serde(default)]
    pub base_subdomain: String,

    /// Whether custom domain mode is enabled.
    #[serde(default)]
    pub custom_domain_enabled: bool,

    // --- Persistent tunnel state (auto-managed) ---

    /// UUID of the persistent tunnel created via API.
    #[serde(default)]
    pub tunnel_id: String,

    /// Connector token for the persistent tunnel.
    #[serde(default)]
    pub tunnel_token_stored: String,

    /// DNS record ID for the wildcard CNAME.
    #[serde(default)]
    pub wildcard_record_id: String,

    /// Cached zone name (e.g. "example.com").
    #[serde(default)]
    pub zone_name: String,
}

impl Settings {
    pub fn load() -> Self {
        let path = settings_path();
        match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, &json)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn has_token(&self) -> bool {
        !self.tunnel_token.trim().is_empty()
    }

    /// Returns true if custom domain mode is enabled and all required fields are set.
    pub fn has_api_config(&self) -> bool {
        self.custom_domain_enabled && self.api_fields_complete()
    }

    /// Returns true if API credentials are fully configured (regardless of toggle).
    pub fn api_fields_complete(&self) -> bool {
        !self.api_token.trim().is_empty()
            && !self.account_id.trim().is_empty()
            && !self.zone_id.trim().is_empty()
            && !self.base_subdomain.trim().is_empty()
    }

    /// Returns true if a persistent tunnel has been provisioned.
    pub fn has_tunnel(&self) -> bool {
        !self.tunnel_id.is_empty() && !self.tunnel_token_stored.is_empty()
    }

    /// Clear persistent tunnel state (e.g. after external deletion).
    pub fn clear_tunnel(&mut self) {
        self.tunnel_id.clear();
        self.tunnel_token_stored.clear();
        self.wildcard_record_id.clear();
        self.zone_name.clear();
    }
}

fn settings_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("cfproxy")
        .join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_no_token() {
        let s = Settings::default();
        assert!(!s.has_token());
        assert!(!s.has_api_config());
        assert!(!s.has_tunnel());
    }

    #[test]
    fn roundtrip_serialize() {
        let mut s = Settings::default();
        s.api_token = "tok".into();
        s.account_id = "acc".into();
        s.zone_id = "zone".into();
        s.base_subdomain = "tunnel".into();
        s.custom_domain_enabled = true;
        let json = serde_json::to_string(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert!(loaded.has_api_config());
        assert_eq!(loaded.base_subdomain, "tunnel");
    }

    #[test]
    fn api_fields_require_base_subdomain() {
        let mut s = Settings::default();
        s.api_token = "tok".into();
        s.account_id = "acc".into();
        s.zone_id = "zone".into();
        // Missing base_subdomain
        assert!(!s.api_fields_complete());
        s.base_subdomain = "tunnel".into();
        assert!(s.api_fields_complete());
    }

    #[test]
    fn has_api_config_requires_enabled() {
        let mut s = Settings::default();
        s.api_token = "tok".into();
        s.account_id = "acc".into();
        s.zone_id = "zone".into();
        s.base_subdomain = "tunnel".into();
        assert!(!s.has_api_config());
        s.custom_domain_enabled = true;
        assert!(s.has_api_config());
    }

    #[test]
    fn toggle_preserves_credentials() {
        let mut s = Settings::default();
        s.api_token = "tok".into();
        s.account_id = "acc".into();
        s.zone_id = "zone".into();
        s.base_subdomain = "tunnel".into();
        s.custom_domain_enabled = true;
        assert!(s.has_api_config());
        s.custom_domain_enabled = false;
        assert!(!s.has_api_config());
        assert!(s.api_fields_complete());
        assert_eq!(s.api_token, "tok");
    }

    #[test]
    fn has_tunnel_checks_id_and_token() {
        let mut s = Settings::default();
        assert!(!s.has_tunnel());
        s.tunnel_id = "uuid".into();
        assert!(!s.has_tunnel());
        s.tunnel_token_stored = "token".into();
        assert!(s.has_tunnel());
    }

    #[test]
    fn clear_tunnel_wipes_state() {
        let mut s = Settings::default();
        s.tunnel_id = "uuid".into();
        s.tunnel_token_stored = "token".into();
        s.wildcard_record_id = "rec".into();
        s.zone_name = "example.com".into();
        s.clear_tunnel();
        assert!(!s.has_tunnel());
        assert!(s.zone_name.is_empty());
    }

    #[test]
    fn backward_compat_old_json() {
        let json = r#"{"tunnel_token":"old-token"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!(s.has_token());
        assert!(!s.has_api_config());
        assert!(!s.custom_domain_enabled);
        assert!(s.base_subdomain.is_empty());
    }

    #[test]
    fn backward_compat_hostname_field_ignored() {
        // Old settings had "hostname", new has "base_subdomain". Old field
        // is silently ignored (serde default), no crash.
        let json = r#"{"api_token":"t","account_id":"a","zone_id":"z","hostname":"old"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!(s.base_subdomain.is_empty());
    }
}
