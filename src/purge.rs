use std::io::{self, BufRead, Write};

use crate::cloudflare::{CloudflareApi, DnsRecordInfo, TunnelInfo};
use crate::error::{Error, Result};
use crate::settings::Settings;

// ---------------------------------------------------------------------------
// Confirmer trait -- abstracts stdin for testability
// ---------------------------------------------------------------------------

trait Confirmer {
    fn confirm(&mut self, msg: &str, default: bool) -> bool;
}

struct StdioConfirmer {
    reader: io::BufReader<io::Stdin>,
}

impl StdioConfirmer {
    fn new() -> Self {
        Self {
            reader: io::BufReader::new(io::stdin()),
        }
    }
}

impl Confirmer for StdioConfirmer {
    fn confirm(&mut self, msg: &str, default: bool) -> bool {
        let hint = if default { "Y/n" } else { "y/N" };
        eprint!("  {} [{}]: ", msg, hint);
        let _ = io::stderr().flush();
        let mut buf = String::new();
        let _ = self.reader.read_line(&mut buf);
        let input = buf.trim().to_lowercase();
        if input.is_empty() {
            default
        } else {
            input.starts_with('y')
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub async fn run_purge() -> Result<()> {
    let mut confirmer = StdioConfirmer::new();
    run_purge_inner(&mut confirmer).await
}

// ---------------------------------------------------------------------------
// Core purge logic -- uses Confirmer trait for testability
// ---------------------------------------------------------------------------

async fn run_purge_inner(confirmer: &mut dyn Confirmer) -> Result<()> {
    eprintln!();
    eprintln!("  \x1b[1;36mcfproxy Purge\x1b[0m");
    eprintln!("  \x1b[90m─────────────\x1b[0m");
    eprintln!();

    let settings = Settings::load();
    if !settings.api_fields_complete() {
        eprintln!("  \x1b[31m✗ API configuration required.\x1b[0m");
        eprintln!("    Run \x1b[1mcfproxy --setup\x1b[0m first to configure your Cloudflare credentials.");
        return Err(Error::Tunnel(
            "purge requires API configuration (run cfproxy --setup)".into(),
        ));
    }

    let api = CloudflareApi::new(&settings.api_token);

    // --- Stale tunnels ---
    eprint!("  Fetching tunnels... ");
    let tunnels = api.list_tunnels(&settings.account_id).await?;
    let cfproxy_tunnels: Vec<&TunnelInfo> = tunnels
        .iter()
        .filter(|t| t.name.starts_with("cfproxy-"))
        .collect();
    eprintln!("\x1b[32m✓\x1b[0m Found {} cfproxy tunnel(s)", cfproxy_tunnels.len());

    let stale_tunnels: Vec<&TunnelInfo> = cfproxy_tunnels
        .iter()
        .copied()
        .filter(|t| {
            // Skip the currently-configured tunnel
            if settings.has_tunnel() && t.id == settings.tunnel_id {
                return false;
            }
            // Include inactive or deleted tunnels
            !matches!(t.status.as_deref(), Some("active"))
        })
        .collect();

    let mut tunnels_deleted = 0u32;
    if stale_tunnels.is_empty() {
        eprintln!("  \x1b[32m✓\x1b[0m No stale tunnels found");
    } else {
        eprintln!();
        eprintln!(
            "  \x1b[33m⚠ Found {} stale tunnel(s):\x1b[0m",
            stale_tunnels.len()
        );
        for t in &stale_tunnels {
            let status = t.status.as_deref().unwrap_or("unknown");
            eprintln!(
                "    - {} ({}...) [{}]",
                t.name,
                &t.id[..8.min(t.id.len())],
                status
            );
        }
        eprintln!();

        if confirmer.confirm("Delete these stale tunnels?", false) {
            for t in &stale_tunnels {
                eprint!("  Deleting tunnel {}... ", t.name);
                match api.delete_tunnel(&settings.account_id, &t.id).await {
                    Ok(()) => {
                        eprintln!("\x1b[32m✓\x1b[0m");
                        tunnels_deleted += 1;
                    }
                    Err(e) => eprintln!("\x1b[31m✗\x1b[0m {}", e),
                }
            }
        } else {
            eprintln!("  Skipped tunnel deletion.");
        }
    }
    eprintln!();

    // --- Orphaned DNS records ---
    eprint!("  Fetching DNS records... ");
    let dns_records = api.list_dns_records(&settings.zone_id).await?;

    // Find CNAME records pointing to *.cfargotunnel.com
    let tunnel_cnames: Vec<&DnsRecordInfo> = dns_records
        .iter()
        .filter(|r| r.record_type == "CNAME" && r.content.ends_with(".cfargotunnel.com"))
        .collect();
    eprintln!(
        "\x1b[32m✓\x1b[0m Found {} tunnel CNAME(s)",
        tunnel_cnames.len()
    );

    // Build set of existing tunnel IDs for quick lookup
    let active_tunnel_ids: Vec<&str> = tunnels
        .iter()
        .filter(|t| t.status.as_deref() != Some("deleted"))
        .map(|t| t.id.as_str())
        .collect();

    let orphaned_cnames: Vec<&DnsRecordInfo> = tunnel_cnames
        .iter()
        .copied()
        .filter(|r| {
            // Skip the currently-configured wildcard record
            if settings.has_tunnel() && r.id == settings.wildcard_record_id {
                return false;
            }
            // Extract tunnel ID from content (format: <tunnel-id>.cfargotunnel.com)
            let tunnel_id = r.content.strip_suffix(".cfargotunnel.com").unwrap_or("");
            !tunnel_id.is_empty() && !active_tunnel_ids.contains(&tunnel_id)
        })
        .collect();

    let mut dns_deleted = 0u32;
    if orphaned_cnames.is_empty() {
        eprintln!("  \x1b[32m✓\x1b[0m No orphaned DNS records found");
    } else {
        eprintln!();
        eprintln!(
            "  \x1b[33m⚠ Found {} orphaned DNS record(s):\x1b[0m",
            orphaned_cnames.len()
        );
        for r in &orphaned_cnames {
            eprintln!("    - {} CNAME → {}", r.name, r.content);
        }
        eprintln!();

        if confirmer.confirm("Delete these orphaned DNS records?", false) {
            for r in &orphaned_cnames {
                eprint!("  Deleting DNS record {}... ", r.name);
                match api.delete_dns_record(&settings.zone_id, &r.id).await {
                    Ok(()) => {
                        eprintln!("\x1b[32m✓\x1b[0m");
                        dns_deleted += 1;
                    }
                    Err(e) => eprintln!("\x1b[31m✗\x1b[0m {}", e),
                }
            }
        } else {
            eprintln!("  Skipped DNS record deletion.");
        }
    }

    // --- Summary ---
    eprintln!();
    eprintln!("  \x1b[1;36m── Summary ──\x1b[0m");
    eprintln!();
    eprintln!(
        "  Tunnels deleted:     {}",
        tunnels_deleted
    );
    eprintln!(
        "  DNS records deleted:  {}",
        dns_deleted
    );
    eprintln!();

    Ok(())
}

// ---------------------------------------------------------------------------
// Filtering helpers (extracted for unit testing)
// ---------------------------------------------------------------------------

#[cfg(test)]
/// Identify cfproxy tunnels that are stale (not the current one, not active).
fn find_stale_tunnels<'a>(
    tunnels: &'a [TunnelInfo],
    current_tunnel_id: Option<&str>,
) -> Vec<&'a TunnelInfo> {
    tunnels
        .iter()
        .filter(|t| t.name.starts_with("cfproxy-"))
        .filter(|t| {
            if let Some(current) = current_tunnel_id {
                if t.id == current {
                    return false;
                }
            }
            !matches!(t.status.as_deref(), Some("active"))
        })
        .collect()
}

#[cfg(test)]
/// Identify orphaned DNS CNAME records pointing to non-existent tunnels.
fn find_orphaned_cnames<'a>(
    records: &'a [DnsRecordInfo],
    active_tunnel_ids: &[&str],
    current_wildcard_id: Option<&str>,
) -> Vec<&'a DnsRecordInfo> {
    records
        .iter()
        .filter(|r| r.record_type == "CNAME" && r.content.ends_with(".cfargotunnel.com"))
        .filter(|r| {
            if let Some(wid) = current_wildcard_id {
                if r.id == wid {
                    return false;
                }
            }
            let tunnel_id = r.content.strip_suffix(".cfargotunnel.com").unwrap_or("");
            !tunnel_id.is_empty() && !active_tunnel_ids.contains(&tunnel_id)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tunnel(id: &str, name: &str, status: Option<&str>) -> TunnelInfo {
        TunnelInfo {
            id: id.to_string(),
            name: name.to_string(),
            status: status.map(String::from),
        }
    }

    fn make_dns(id: &str, name: &str, rtype: &str, content: &str) -> DnsRecordInfo {
        DnsRecordInfo {
            id: id.to_string(),
            name: name.to_string(),
            record_type: rtype.to_string(),
            content: content.to_string(),
        }
    }

    // --- Tunnel filtering tests ---

    #[test]
    fn find_stale_tunnels_skips_non_cfproxy() {
        let tunnels = vec![
            make_tunnel("t1", "other-tunnel", Some("inactive")),
            make_tunnel("t2", "cfproxy-sandbox", Some("inactive")),
        ];
        let stale = find_stale_tunnels(&tunnels, None);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].id, "t2");
    }

    #[test]
    fn find_stale_tunnels_skips_current() {
        let tunnels = vec![
            make_tunnel("current-id", "cfproxy-sandbox", Some("inactive")),
            make_tunnel("other-id", "cfproxy-old", Some("inactive")),
        ];
        let stale = find_stale_tunnels(&tunnels, Some("current-id"));
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].id, "other-id");
    }

    #[test]
    fn find_stale_tunnels_skips_active() {
        let tunnels = vec![
            make_tunnel("t1", "cfproxy-sandbox", Some("active")),
            make_tunnel("t2", "cfproxy-old", Some("inactive")),
            make_tunnel("t3", "cfproxy-deleted", Some("deleted")),
        ];
        let stale = find_stale_tunnels(&tunnels, None);
        assert_eq!(stale.len(), 2);
        assert!(stale.iter().any(|t| t.id == "t2"));
        assert!(stale.iter().any(|t| t.id == "t3"));
    }

    #[test]
    fn find_stale_tunnels_includes_no_status() {
        let tunnels = vec![make_tunnel("t1", "cfproxy-sandbox", None)];
        let stale = find_stale_tunnels(&tunnels, None);
        assert_eq!(stale.len(), 1);
    }

    #[test]
    fn find_stale_tunnels_empty() {
        let tunnels: Vec<TunnelInfo> = vec![];
        let stale = find_stale_tunnels(&tunnels, None);
        assert!(stale.is_empty());
    }

    // --- DNS filtering tests ---

    #[test]
    fn find_orphaned_cnames_identifies_orphans() {
        let records = vec![
            make_dns("r1", "*.example.com", "CNAME", "dead-id.cfargotunnel.com"),
            make_dns("r2", "*.example.com", "CNAME", "alive-id.cfargotunnel.com"),
            make_dns("r3", "www.example.com", "A", "1.2.3.4"),
        ];
        let active = vec!["alive-id"];
        let orphans = find_orphaned_cnames(&records, &active, None);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].id, "r1");
    }

    #[test]
    fn find_orphaned_cnames_skips_current_wildcard() {
        let records = vec![
            make_dns("current-wc", "*.example.com", "CNAME", "dead-id.cfargotunnel.com"),
        ];
        let orphans = find_orphaned_cnames(&records, &[], Some("current-wc"));
        assert!(orphans.is_empty());
    }

    #[test]
    fn find_orphaned_cnames_ignores_non_tunnel_cnames() {
        let records = vec![
            make_dns("r1", "app.example.com", "CNAME", "some-other.target.com"),
        ];
        let orphans = find_orphaned_cnames(&records, &[], None);
        assert!(orphans.is_empty());
    }

    #[test]
    fn find_orphaned_cnames_empty() {
        let records: Vec<DnsRecordInfo> = vec![];
        let orphans = find_orphaned_cnames(&records, &[], None);
        assert!(orphans.is_empty());
    }

    // --- Config requirement test ---

    #[test]
    fn purge_requires_api_config() {
        let settings = Settings::default();
        assert!(!settings.api_fields_complete());
    }
}
