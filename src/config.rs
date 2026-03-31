use std::path::PathBuf;

use crate::cli::Args;
use crate::mock::MockRule;

pub struct Config {
    pub port: u16,
    pub binary_path: Option<PathBuf>,
    pub cache_dir: PathBuf,
    pub auto_download: bool,
    pub auth: Option<(String, String)>,
    pub mock_rules: Vec<MockRule>,
    pub host: Option<String>,
    pub quick: bool,
    pub allow_ips: Vec<String>,
}

impl Config {
    pub fn from_args(args: Args) -> Self {
        let cache_dir = args
            .cache_dir
            .unwrap_or_else(default_cache_dir);

        let auth = args.auth.and_then(|s| {
            let mut parts = s.splitn(2, ':');
            let user = parts.next()?.to_string();
            let pass = parts.next()?.to_string();
            Some((user, pass))
        });

        let mock_rules: Vec<MockRule> = args
            .mock
            .iter()
            .filter_map(|s| MockRule::parse(s))
            .collect();

        Self {
            port: args.port,
            binary_path: args.cloudflared_path,
            cache_dir,
            auto_download: !args.no_download,
            auth,
            mock_rules,
            host: args.host,
            quick: args.quick,
            allow_ips: args.allow_ip,
        }
    }
}

pub fn default_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("cfproxy")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cache_dir_is_under_cfproxy() {
        let dir = default_cache_dir();
        assert!(dir.ends_with("cfproxy"));
    }

    #[test]
    fn from_args_respects_no_download() {
        let args = Args {
            port: 8080,
            cloudflared_path: None,
            no_download: true,
            cache_dir: Some(PathBuf::from("/tmp/test")),
            auth: None,
            mock: Vec::new(),
            host: None,
            quick: false,
            setup: false,
            purge: false,
            doctor: false,
            update: false,
            allow_ip: Vec::new(),
        };
        let config = Config::from_args(args);
        assert!(!config.auto_download);
        assert_eq!(config.port, 8080);
    }
}
