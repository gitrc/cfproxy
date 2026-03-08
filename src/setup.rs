use std::io::{self, BufRead, Write};

use crate::cloudflare::{self, CloudflareApi};
use crate::error::{Error, Result};
use crate::settings::Settings;

// ---------------------------------------------------------------------------
// Prompter trait — abstracts stdin/stdout for testability
// ---------------------------------------------------------------------------

trait Prompter {
    fn print(&mut self, msg: &str);
    fn println(&mut self, msg: &str);
    fn prompt(&mut self, label: &str, default: &str) -> String;
    fn prompt_secret(&mut self, label: &str) -> String;
    fn confirm(&mut self, msg: &str, default: bool) -> bool;
    fn choose(&mut self, label: &str, options: &[(String, String)]) -> usize;
}

struct StdioPrompter {
    reader: io::BufReader<io::Stdin>,
}

impl StdioPrompter {
    fn new() -> Self {
        Self {
            reader: io::BufReader::new(io::stdin()),
        }
    }

    fn read_line(&mut self) -> String {
        let mut buf = String::new();
        let _ = self.reader.read_line(&mut buf);
        buf.trim().to_string()
    }
}

impl Prompter for StdioPrompter {
    fn print(&mut self, msg: &str) {
        print!("{}", msg);
        let _ = io::stdout().flush();
    }

    fn println(&mut self, msg: &str) {
        println!("{}", msg);
    }

    fn prompt(&mut self, label: &str, default: &str) -> String {
        if default.is_empty() {
            print!("  {}: ", label);
        } else {
            print!("  {} [{}]: ", label, default);
        }
        let _ = io::stdout().flush();
        let input = self.read_line();
        if input.is_empty() {
            default.to_string()
        } else {
            input
        }
    }

    fn prompt_secret(&mut self, label: &str) -> String {
        print!("  {}: ", label);
        let _ = io::stdout().flush();
        // No echo suppression — token is typically pasted, not typed
        self.read_line()
    }

    fn confirm(&mut self, msg: &str, default: bool) -> bool {
        let hint = if default { "Y/n" } else { "y/N" };
        print!("  {} [{}]: ", msg, hint);
        let _ = io::stdout().flush();
        let input = self.read_line().to_lowercase();
        if input.is_empty() {
            default
        } else {
            input.starts_with('y')
        }
    }

    fn choose(&mut self, label: &str, options: &[(String, String)]) -> usize {
        println!();
        for (i, (id, name)) in options.iter().enumerate() {
            let short_id = if id.len() > 12 {
                format!("{}...", &id[..9])
            } else {
                id.clone()
            };
            println!("    {}) {} ({})", i + 1, name, short_id);
        }
        println!();
        loop {
            let default = if options.len() == 1 { "1" } else { "" };
            let input = self.prompt(label, default);
            if let Ok(n) = input.parse::<usize>() {
                if n >= 1 && n <= options.len() {
                    return n - 1;
                }
            }
            println!("  Please enter a number between 1 and {}", options.len());
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub async fn run_setup_wizard() -> Result<()> {
    let mut prompter = StdioPrompter::new();
    run_wizard(&mut prompter).await
}

// ---------------------------------------------------------------------------
// Wizard flow — uses Prompter trait for testability
// ---------------------------------------------------------------------------

async fn run_wizard(p: &mut dyn Prompter) -> Result<()> {
    p.println("");
    p.println("  \x1b[1;36mcfproxy Custom Domain Setup\x1b[0m");
    p.println("  \x1b[90m───────────────────────────\x1b[0m");
    p.println("");

    // Check for existing setup
    let mut settings = Settings::load();
    if settings.has_tunnel() {
        p.println("  \x1b[33m⚠ An existing tunnel is already configured.\x1b[0m");
        p.println(&format!(
            "    Tunnel: {}",
            &settings.tunnel_id[..8.min(settings.tunnel_id.len())]
        ));
        if !settings.zone_name.is_empty() {
            p.println(&format!(
                "    Domain: *-{}.{}",
                settings.base_subdomain, settings.zone_name
            ));
        }
        p.println("");
        if !p.confirm("Re-run setup? This will create a new tunnel", false) {
            p.println("");
            p.println("  Setup cancelled. Existing configuration unchanged.");
            return Ok(());
        }
        p.println("");
    }

    // Step 1: API Token
    p.println("  \x1b[1;36mStep 1: Create a Cloudflare API Token\x1b[0m");
    p.println("");
    p.println("  Go to: \x1b[4mhttps://dash.cloudflare.com/profile/api-tokens\x1b[0m");
    p.println("");

    // Try to open browser
    open_browser("https://dash.cloudflare.com/profile/api-tokens");

    p.println("  Click \x1b[1mCreate Token\x1b[0m, then \x1b[1mCreate Custom Token\x1b[0m.");
    p.println("  Add these permissions:");
    p.println("    \x1b[36m•\x1b[0m Account → Account Settings → Read");
    p.println("    \x1b[36m•\x1b[0m Account → Cloudflare Tunnel → Edit");
    p.println("    \x1b[36m•\x1b[0m Zone → Zone → Read");
    p.println("    \x1b[36m•\x1b[0m Zone → DNS → Edit");
    p.println("");
    p.println("  Under \x1b[1mAccount Resources\x1b[0m, select your account.");
    p.println("  Under \x1b[1mZone Resources\x1b[0m, select All zones (or a specific zone).");
    p.println("");

    let token = p.prompt_secret("Paste your API token");
    if token.is_empty() {
        return Err(Error::Tunnel("no token provided".into()));
    }

    // Validate token
    p.print("  Validating token... ");
    let api = CloudflareApi::new(&token);
    match api.validate_token().await {
        Ok(true) => p.println("\x1b[32m✓ Valid\x1b[0m"),
        _ => {
            p.println("\x1b[31m✗ Invalid\x1b[0m");
            p.println("");
            p.println("  The token could not be verified. Check that you copied");
            p.println("  the full token and that it has the correct permissions.");
            return Err(Error::Tunnel("API token validation failed".into()));
        }
    }
    p.println("");

    // Step 2: Select Account
    p.println("  \x1b[1;36mStep 2: Select Account\x1b[0m");
    p.print("  Fetching accounts... ");
    let accounts = api.list_accounts().await?;
    if accounts.is_empty() {
        p.println("\x1b[31m✗ No accounts found\x1b[0m");
        p.println("");
        p.println("  Your API token doesn't have access to any accounts.");
        p.println("  Make sure 'Account Resources' includes your account.");
        return Err(Error::Tunnel("no accounts accessible".into()));
    }
    p.println(&format!("\x1b[32m✓\x1b[0m Found {}", accounts.len()));

    let account_idx = if accounts.len() == 1 {
        p.println(&format!("  Using account: \x1b[1m{}\x1b[0m", accounts[0].1));
        0
    } else {
        p.choose("Select account", &accounts)
    };
    let (account_id, account_name) = &accounts[account_idx];
    p.println("");

    // Step 3: Select Domain
    p.println("  \x1b[1;36mStep 3: Select Domain\x1b[0m");
    p.print("  Fetching zones... ");
    let zones = api.list_zones().await?;
    if zones.is_empty() {
        p.println("\x1b[31m✗ No domains found\x1b[0m");
        p.println("");
        p.println("  Your API token doesn't have access to any domains.");
        p.println("  Make sure you have a domain added to Cloudflare and");
        p.println("  'Zone Resources' includes it.");
        return Err(Error::Tunnel("no zones accessible".into()));
    }
    p.println(&format!("\x1b[32m✓\x1b[0m Found {}", zones.len()));

    let zone_idx = if zones.len() == 1 {
        p.println(&format!("  Using domain: \x1b[1m{}\x1b[0m", zones[0].1));
        0
    } else {
        p.choose("Select domain", &zones)
    };
    let (zone_id, zone_name) = &zones[zone_idx];
    p.println("");

    // Step 4: Base Prefix
    p.println("  \x1b[1;36mStep 4: Base Prefix\x1b[0m");
    p.println("");
    p.println(&format!(
        "  Tunnels use the format \x1b[1m{{name}}-{{prefix}}.{}\x1b[0m",
        zone_name
    ));
    p.println(&format!(
        "  For example, with \"sandbox\" you get: myapp-sandbox.{}",
        zone_name
    ));
    p.println("");

    let base_subdomain = p.prompt("Base prefix", "tunnel");
    if base_subdomain.is_empty() {
        return Err(Error::Tunnel("base subdomain is required".into()));
    }
    // Validate: only lowercase alphanumeric and hyphens, no leading/trailing hyphens
    let valid = !base_subdomain.is_empty()
        && base_subdomain
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !base_subdomain.starts_with('-')
        && !base_subdomain.ends_with('-');
    if !valid {
        return Err(Error::Tunnel(
            "subdomain must be lowercase alphanumeric with hyphens (e.g. \"sandbox\", \"my-tunnel\")"
                .into(),
        ));
    }
    p.println("");

    // Check for conflicting DNS records before provisioning
    p.print("  Checking DNS for conflicts... ");
    {
        let check_api = CloudflareApi::new(&token);
        let wildcard = format!("*.{}", zone_name);

        let wildcard_exists = check_api.dns_records_exist(zone_id, &wildcard).await.unwrap_or(false);

        if wildcard_exists {
            p.println("\x1b[33m⚠\x1b[0m");
            p.println("");
            p.println(&format!(
                "  \x1b[33m⚠ Wildcard DNS record already exists: {}\x1b[0m",
                wildcard
            ));
            p.println("  Existing specific records (www, mail, etc.) are NOT affected");
            p.println("  — they take priority over the wildcard.");
            p.println("");
            if !p.confirm("Continue? (will update wildcard to point to new tunnel)", false) {
                p.println("");
                p.println("  Setup cancelled.");
                return Ok(());
            }
        } else {
            p.println("\x1b[32m✓\x1b[0m No conflicts");
        }
    }
    p.println("");

    // Step 5: Provision
    p.println("  \x1b[1;36mStep 5: Provisioning\x1b[0m");
    p.println("");

    // Clear old tunnel state if re-provisioning
    settings.clear_tunnel();

    // Save credentials
    settings.api_token = token;
    settings.account_id = account_id.clone();
    settings.zone_id = zone_id.clone();
    settings.zone_name = zone_name.clone();
    settings.base_subdomain = base_subdomain.clone();
    settings.custom_domain_enabled = true;

    // Provision tunnel + wildcard DNS
    p.print("  Creating tunnel... ");
    match cloudflare::ensure_tunnel(&mut settings).await {
        Ok(()) => {
            p.println("\x1b[32m✓\x1b[0m");
        }
        Err(e) => {
            p.println("\x1b[31m✗\x1b[0m");
            p.println(&format!("  Error: {}", e));
            p.println("");
            p.println("  Check your token permissions and try again:");
            p.println("    cfproxy --setup");
            return Err(e);
        }
    }

    // Save settings
    p.print("  Saving settings... ");
    match settings.save() {
        Ok(()) => p.println("\x1b[32m✓\x1b[0m"),
        Err(e) => {
            p.println("\x1b[31m✗\x1b[0m");
            return Err(Error::Tunnel(format!("failed to save settings: {}", e)));
        }
    }

    // Summary
    p.println("");
    p.println("  \x1b[1;36m── Summary ──\x1b[0m");
    p.println("");
    p.println(&format!(
        "  \x1b[32m✓\x1b[0m Account   {}",
        account_name
    ));
    p.println(&format!(
        "  \x1b[32m✓\x1b[0m Domain    {}",
        zone_name
    ));
    p.println(&format!(
        "  \x1b[32m✓\x1b[0m Tunnel    cfproxy-{} ({}...)",
        base_subdomain,
        &settings.tunnel_id[..8.min(settings.tunnel_id.len())]
    ));
    p.println(&format!(
        "  \x1b[32m✓\x1b[0m DNS       *.{}",
        zone_name
    ));
    p.println("");
    p.println("  \x1b[1mSetup complete!\x1b[0m Run cfproxy to start:");
    p.println("");
    p.println(&format!(
        "    cfproxy 3000                    \x1b[90m→ <random>-{}.{}\x1b[0m",
        base_subdomain, zone_name
    ));
    p.println(&format!(
        "    cfproxy 3000 --host myapp       \x1b[90m→ myapp-{}.{}\x1b[0m",
        base_subdomain, zone_name
    ));
    p.println("");

    Ok(())
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockPrompter {
        outputs: Vec<String>,
        inputs: Vec<String>,
        input_idx: usize,
    }

    impl MockPrompter {
        fn new(inputs: Vec<&str>) -> Self {
            Self {
                outputs: Vec::new(),
                inputs: inputs.into_iter().map(String::from).collect(),
                input_idx: 0,
            }
        }

        fn next_input(&mut self) -> String {
            let val = self
                .inputs
                .get(self.input_idx)
                .cloned()
                .unwrap_or_default();
            self.input_idx += 1;
            val
        }

        #[allow(dead_code)]
        fn output_contains(&self, needle: &str) -> bool {
            self.outputs.iter().any(|line| line.contains(needle))
        }
    }

    impl Prompter for MockPrompter {
        fn print(&mut self, msg: &str) {
            self.outputs.push(msg.to_string());
        }

        fn println(&mut self, msg: &str) {
            self.outputs.push(msg.to_string());
        }

        fn prompt(&mut self, _label: &str, default: &str) -> String {
            let input = self.next_input();
            if input.is_empty() {
                default.to_string()
            } else {
                input
            }
        }

        fn prompt_secret(&mut self, _label: &str) -> String {
            self.next_input()
        }

        fn confirm(&mut self, _msg: &str, default: bool) -> bool {
            let input = self.next_input();
            if input.is_empty() {
                default
            } else {
                input.starts_with('y')
            }
        }

        fn choose(&mut self, _label: &str, options: &[(String, String)]) -> usize {
            let input = self.next_input();
            let n: usize = input.parse().unwrap_or(1);
            (n - 1).min(options.len().saturating_sub(1))
        }
    }

    #[test]
    fn mock_prompter_returns_defaults() {
        let mut p = MockPrompter::new(vec![""]);
        assert_eq!(p.prompt("Name", "default"), "default");
    }

    #[test]
    fn mock_prompter_returns_input() {
        let mut p = MockPrompter::new(vec!["custom"]);
        assert_eq!(p.prompt("Name", "default"), "custom");
    }

    #[test]
    fn mock_prompter_confirm_default_yes() {
        let mut p = MockPrompter::new(vec![""]);
        assert!(p.confirm("Continue?", true));
    }

    #[test]
    fn mock_prompter_confirm_explicit_no() {
        let mut p = MockPrompter::new(vec!["n"]);
        assert!(!p.confirm("Continue?", true));
    }

    #[test]
    fn mock_prompter_choose() {
        let options = vec![
            ("id1".into(), "Option A".into()),
            ("id2".into(), "Option B".into()),
        ];
        let mut p = MockPrompter::new(vec!["2"]);
        assert_eq!(p.choose("Pick", &options), 1);
    }

    #[test]
    fn empty_token_is_error() {
        // Can't easily test the full async wizard without a mock API,
        // but we can verify the prompter plumbing works
        let mut p = MockPrompter::new(vec![""]);
        let token = p.prompt_secret("Token");
        assert!(token.is_empty());
    }

    #[test]
    fn open_browser_does_not_panic() {
        // Use a non-existent scheme so it doesn't actually open a browser window
        open_browser("cfproxy-test://noop");
    }
}
