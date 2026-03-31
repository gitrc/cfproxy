use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "cfproxy",
    version,
    about = "Expose localhost services via Cloudflare tunnel"
)]
pub struct Args {
    /// Local port to expose (not required with --setup)
    #[arg(default_value_t = 0)]
    pub port: u16,

    /// Path to cloudflared binary (skips auto-download)
    #[arg(long, env = "CFPROXY_CLOUDFLARED_PATH")]
    pub cloudflared_path: Option<PathBuf>,

    /// Disable automatic download of cloudflared
    #[arg(long, env = "CFPROXY_NO_DOWNLOAD")]
    pub no_download: bool,

    /// Enable HTTP Basic Auth (format: user:pass)
    #[arg(long, env = "CFPROXY_AUTH")]
    pub auth: Option<String>,

    /// Directory to cache the cloudflared binary
    #[arg(long, env = "CFPROXY_CACHE_DIR")]
    pub cache_dir: Option<PathBuf>,

    /// Mock a response (format: [METHOD] /path:status[:body])
    /// Can be specified multiple times
    #[arg(long, env = "CFPROXY_MOCK", value_delimiter = ',')]
    pub mock: Vec<String>,

    /// Custom subdomain for this run (e.g. "myapp" → myapp.tunnel.example.com)
    /// Requires custom domain to be configured in settings (press S in UI)
    #[arg(long, env = "CFPROXY_HOST")]
    pub host: Option<String>,

    /// Force quick tunnel mode (trycloudflare.com) even if custom domain is configured
    #[arg(long, env = "CFPROXY_QUICK")]
    pub quick: bool,

    /// Run interactive setup wizard for custom domain configuration
    #[arg(long)]
    pub setup: bool,

    /// Find and clean stale/orphaned tunnels and DNS records
    #[arg(long)]
    pub purge: bool,

    /// Run diagnostic checks (settings, binary, network, API)
    #[arg(long)]
    pub doctor: bool,

    /// Update the cached cloudflared binary to the latest version
    #[arg(long)]
    pub update: bool,

    /// Only allow requests from these IPs (checked via CF-Connecting-IP header).
    /// Can be specified multiple times. When set, all other IPs get 403.
    #[arg(long = "allow-ip", env = "CFPROXY_ALLOW_IP", value_delimiter = ',')]
    pub allow_ip: Vec<String>,
}
