mod cli;
mod cloudflare;
mod cloudflared;
mod config;
mod diff;
mod doctor;
mod error;
mod event;
mod har;
mod metrics;
mod mock;
mod proxy;
mod purge;
mod settings;
mod setup;
mod tunnel;
mod qr;
mod ui;

use clap::Parser;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> error::Result<()> {
    let args = cli::Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    // --setup: run interactive wizard and exit
    if args.setup {
        return setup::run_setup_wizard().await;
    }

    // --purge: find and clean stale tunnels/DNS records and exit
    if args.purge {
        return purge::run_purge().await;
    }

    // --doctor: run diagnostic checks and exit
    if args.doctor {
        return doctor::run_doctor(args.cache_dir.clone(), args.cloudflared_path.clone()).await;
    }

    // --update: re-download cloudflared binary and exit
    if args.update {
        let cache_dir = args.cache_dir.unwrap_or_else(config::default_cache_dir);
        return cloudflared::update(cache_dir).await;
    }

    // Port is required for normal operation
    if args.port == 0 {
        eprintln!("error: <PORT> is required (or use --setup for initial configuration, --doctor for diagnostics)");
        std::process::exit(1);
    }

    let config = config::Config::from_args(args);

    // Early validation: --host and --quick are mutually exclusive
    if config.host.is_some() && config.quick {
        eprintln!("error: --host and --quick cannot be used together");
        std::process::exit(1);
    }

    // Early validation: --host requires custom domain config
    if config.host.is_some() {
        let settings = settings::Settings::load();
        if !settings.has_api_config() && !settings.api_fields_complete() {
            return Err(error::Error::Tunnel(
                "--host requires custom domain setup. Run `cfproxy --setup` to configure."
                    .into(),
            ));
        }
    }

    let manager = cloudflared::BinaryManager::new(
        config.cache_dir.clone(),
        config.auto_download,
        config.binary_path.clone(),
    );
    let binary_path = manager.ensure().await?;
    tracing::info!("using cloudflared at {}", binary_path.display());

    let (tx, rx) = mpsc::channel(100);

    // Build mock rules from config
    let mock_rules = mock::new_rules();
    {
        let mut rules = mock_rules.write().await;
        *rules = config.mock_rules;
    }

    // Start local reverse proxy to capture HTTP requests
    let proxy_port = proxy::start(config.port, tx.clone(), config.auth, mock_rules.clone(), config.allow_ips).await?;
    tracing::info!("request logger listening on 127.0.0.1:{}", proxy_port);

    let port = config.port;
    let binary = binary_path.clone();
    let mut saved_settings = settings::Settings::load();

    // Determine tunnel mode
    let (tunnel_token, custom_url, run_hostname) = setup_tunnel_mode(
        &mut saved_settings,
        &config.host,
        config.quick,
        proxy_port,
    )
    .await?;

    tokio::spawn(async move {
        let mut t = tunnel::Tunnel::new();
        if let Err(e) = t
            .start(&binary, proxy_port, tunnel_token.as_deref(), custom_url, tx)
            .await
        {
            tracing::error!("tunnel error: {}", e);
        }
    });

    let result = ui::run(port, rx, mock_rules).await;

    // Remove our hostname from ingress on exit
    if let Some(hostname) = &run_hostname {
        let settings = settings::Settings::load();
        if settings.has_tunnel() {
            if let Err(e) = cloudflare::clear_ingress(&settings, hostname).await {
                tracing::warn!("failed to clear ingress on exit: {}", e);
            }
        }
    }

    result
}

/// Determine tunnel mode and return (token, custom_url, hostname) for cloudflared.
/// The hostname is returned so we can remove it from ingress on exit.
async fn setup_tunnel_mode(
    settings: &mut settings::Settings,
    host: &Option<String>,
    quick: bool,
    proxy_port: u16,
) -> error::Result<(Option<String>, Option<String>, Option<String>)> {
    // --quick forces trycloudflare.com regardless of settings
    if quick {
        return Ok((None, None, None));
    }

    let use_custom = settings.has_api_config()
        || (host.is_some() && settings.api_fields_complete());

    if use_custom {
        return setup_custom_domain(settings, host, proxy_port).await;
    }

    // Legacy manual token
    if settings.has_token() {
        return Ok((Some(settings.tunnel_token.clone()), None, None));
    }

    // Quick tunnel (default)
    Ok((None, None, None))
}

async fn setup_custom_domain(
    settings: &mut settings::Settings,
    host: &Option<String>,
    proxy_port: u16,
) -> error::Result<(Option<String>, Option<String>, Option<String>)> {
    // Ensure persistent tunnel + wildcard DNS exist
    match cloudflare::ensure_tunnel(settings).await {
        Ok(()) => {}
        Err(e) => {
            tracing::error!(
                "failed to provision tunnel: {} — falling back to quick tunnel",
                e
            );
            return Ok((None, None, None));
        }
    }

    // Determine hostname for this run
    let host_prefix = match host {
        Some(h) => h.clone(),
        None => cloudflare::random_subdomain(),
    };
    let hostname = cloudflare::resolve_hostname(
        &host_prefix,
        &settings.base_subdomain,
        &settings.zone_name,
    );

    // Add our hostname to ingress (merges with existing entries)
    match cloudflare::update_ingress(settings, &hostname, proxy_port).await {
        Ok(()) => {
            let url = format!("https://{}", hostname);
            tracing::info!("custom domain: {}", url);
            Ok((
                Some(settings.tunnel_token_stored.clone()),
                Some(url),
                Some(hostname),
            ))
        }
        Err(e) => {
            tracing::error!(
                "failed to configure ingress: {} — falling back to quick tunnel",
                e
            );
            Ok((None, None, None))
        }
    }
}
