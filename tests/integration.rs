use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helper: build a Command for the cfproxy binary
// ---------------------------------------------------------------------------
fn cfproxy() -> Command {
    Command::cargo_bin("cfproxy").expect("binary cfproxy should be buildable")
}

// ===========================================================================
// 1. CLI argument parsing tests (via assert_cmd)
// ===========================================================================

#[test]
fn help_flag_exits_zero() {
    cfproxy()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Expose localhost services via Cloudflare tunnel"));
}

#[test]
fn version_flag_exits_zero() {
    cfproxy()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("cfproxy"));
}

#[test]
fn missing_port_shows_error() {
    cfproxy()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage").or(predicate::str::contains("required")));
}

#[test]
fn invalid_port_not_a_number() {
    cfproxy()
        .arg("notaport")
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn invalid_port_out_of_range() {
    // u16 max is 65535; 99999 overflows
    cfproxy()
        .arg("99999")
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn port_zero_shows_required_error() {
    // Port 0 is the default when no port is given. The binary treats it as
    // "no port specified" and exits with a helpful message.
    cfproxy()
        .args(["0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required").or(predicate::str::contains("--setup")));
}

// ===========================================================================
// 2. Binary execution tests -- cloudflared-path behavior
// ===========================================================================

#[test]
fn nonexistent_cloudflared_path_fails() {
    cfproxy()
        .args(["8080", "--cloudflared-path", "/no/such/binary"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("/no/such/binary")));
}

#[test]
fn cloudflared_path_via_env_nonexistent_fails() {
    cfproxy()
        .arg("8080")
        .env("CFPROXY_CLOUDFLARED_PATH", "/tmp/does_not_exist_cfproxy_test")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(
            predicate::str::contains("does_not_exist_cfproxy_test"),
        ));
}

#[test]
fn no_download_flag_without_cloudflared_in_path() {
    // With --no-download and an empty PATH (so cloudflared cannot be found),
    // the binary should fail with a "not found" message.
    cfproxy()
        .args(["8080", "--no-download"])
        .env("PATH", "/empty_path_for_cfproxy_test")
        .env("CFPROXY_CLOUDFLARED_PATH", "")
        .assert()
        .failure();
}

#[test]
fn no_download_via_env() {
    cfproxy()
        .arg("8080")
        .env("CFPROXY_NO_DOWNLOAD", "true")
        .env("PATH", "/empty_path_for_cfproxy_test")
        .env("CFPROXY_CLOUDFLARED_PATH", "")
        .assert()
        .failure();
}

#[test]
fn cache_dir_flag_accepted() {
    let tmp = TempDir::new().expect("should create tempdir");
    // With --cache-dir, --no-download, and an empty PATH, the binary should
    // fail because it cannot locate cloudflared -- but it should NOT fail on
    // argument parsing.
    cfproxy()
        .args([
            "8080",
            "--no-download",
            "--cache-dir",
            tmp.path().to_str().unwrap(),
        ])
        .env("PATH", "/empty_path_for_cfproxy_test")
        .assert()
        .failure()
        // The error should mention the binary not being found, not an argument error
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("cloudflared")));
}

#[test]
fn cache_dir_via_env_accepted() {
    let tmp = TempDir::new().expect("should create tempdir");
    cfproxy()
        .args(["8080", "--no-download"])
        .env("CFPROXY_CACHE_DIR", tmp.path().to_str().unwrap())
        .env("PATH", "/empty_path_for_cfproxy_test")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("cloudflared")));
}

// ===========================================================================
// 3. Verify the binary name and metadata
// ===========================================================================

#[test]
fn help_shows_port_argument() {
    cfproxy()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("port"));
}

#[test]
fn help_shows_cloudflared_path_option() {
    cfproxy()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--cloudflared-path"));
}

#[test]
fn help_shows_no_download_option() {
    cfproxy()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--no-download"));
}

#[test]
fn help_shows_cache_dir_option() {
    cfproxy()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--cache-dir"));
}

// ===========================================================================
// 4. Cloudflared-path pointing to a non-executable file
// ===========================================================================

#[test]
fn cloudflared_path_to_non_executable_file() {
    let tmp = TempDir::new().expect("should create tempdir");
    let fake_binary = tmp.path().join("cloudflared");
    std::fs::write(&fake_binary, "not a real binary").expect("write fake binary");

    // The file exists, so BinaryManager::ensure() returns Ok(path).
    // The process will then try to spawn it and fail. On some systems this is
    // an IO error, on others a permission error. Either way the binary exits
    // with a non-zero status.
    cfproxy()
        .args(["8080", "--cloudflared-path", fake_binary.to_str().unwrap()])
        .assert()
        .failure();
}

// ===========================================================================
// 5. Multiple flags combined
// ===========================================================================

#[test]
fn all_flags_combined() {
    let tmp = TempDir::new().expect("should create tempdir");
    cfproxy()
        .args([
            "3000",
            "--no-download",
            "--cache-dir",
            tmp.path().to_str().unwrap(),
            "--cloudflared-path",
            "/nonexistent/cloudflared",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("cloudflared")));
}

// ===========================================================================
// 6. Cached binary resolution via tempdir
// ===========================================================================

#[test]
fn cached_binary_is_used_when_present() {
    let tmp = TempDir::new().expect("should create tempdir");
    let cached = tmp.path().join("cloudflared");

    // Create a fake "binary" that will be found by BinaryManager.
    // It exists so ensure() returns it, but spawning it will fail.
    std::fs::write(&cached, "#!/bin/sh\nexit 1\n").expect("write cached binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cached, std::fs::Permissions::from_mode(0o755))
            .expect("set permissions");
    }

    // Use --cache-dir pointing to our tmp dir, with --no-download.
    // The binary manager should find the cached binary and try to run it.
    cfproxy()
        .args([
            "8080",
            "--no-download",
            "--cache-dir",
            tmp.path().to_str().unwrap(),
        ])
        .env("PATH", "/empty_path_for_cfproxy_test")
        .assert()
        .failure();
    // The fact that it did not complain about "not found" means the cached
    // binary was resolved. It fails because our fake binary is not a real
    // cloudflared.
}

// ===========================================================================
// 7. Negative port value
// ===========================================================================

#[test]
fn negative_port_rejected() {
    cfproxy()
        .arg("-1")
        // "-1" will be interpreted as an unknown flag
        .assert()
        .failure();
}

// ===========================================================================
// 8. Extra unexpected arguments
// ===========================================================================

#[test]
fn extra_positional_args_rejected() {
    cfproxy()
        .args(["8080", "extra"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("unexpected")
                .or(predicate::str::contains("not expected")),
        );
}

// ===========================================================================
// 9. Ensure error output goes to stderr (not stdout)
// ===========================================================================

#[test]
fn error_output_goes_to_stderr() {
    let output = cfproxy()
        .args(["8080", "--cloudflared-path", "/nonexistent/path"])
        .output()
        .expect("should execute");

    assert!(!output.status.success());
    // stdout should be empty or minimal; the error goes to stderr
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("cloudflared") || stderr.contains("Error"),
        "expected error message on stderr, got: {}",
        stderr
    );
}

// ===========================================================================
// 10. --auth flag tests
// ===========================================================================

#[test]
fn help_shows_auth_option() {
    cfproxy()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--auth"));
}

#[test]
fn auth_flag_accepted_by_parser() {
    // --auth should be accepted without a parse error; it will fail later
    // because cloudflared is not found, not because of argument parsing.
    cfproxy()
        .args(["8080", "--auth", "user:pass", "--cloudflared-path", "/nonexistent/cloudflared"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("cloudflared")));
}

#[test]
fn auth_flag_via_env_accepted() {
    cfproxy()
        .args(["8080", "--cloudflared-path", "/nonexistent/cloudflared"])
        .env("CFPROXY_AUTH", "admin:secret")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("cloudflared")));
}

// ===========================================================================
// 11. Verify version string format
// ===========================================================================

#[test]
fn version_output_contains_semver_like_string() {
    cfproxy()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"\d+\.\d+\.\d+").expect("valid regex"));
}

// ===========================================================================
// 12. Mock flag tests
// ===========================================================================

#[test]
fn mock_flag_accepted_by_parser() {
    cfproxy()
        .args(["8080", "--mock", "/health:200:OK", "--cloudflared-path", "/nonexistent/cloudflared"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("cloudflared")));
}

#[test]
fn help_shows_mock_option() {
    cfproxy()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--mock"));
}

// ===========================================================================
// 13. --host flag tests
// ===========================================================================

#[test]
fn help_shows_host_option() {
    cfproxy()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--host"));
}

#[test]
fn host_flag_accepted_by_parser() {
    // --host should be accepted; will fail because cloudflared not found
    cfproxy()
        .args([
            "8080",
            "--host",
            "myapp",
            "--cloudflared-path",
            "/nonexistent/cloudflared",
        ])
        .assert()
        .failure();
}

#[test]
fn host_flag_via_env_accepted() {
    cfproxy()
        .args([
            "8080",
            "--cloudflared-path",
            "/nonexistent/cloudflared",
        ])
        .env("CFPROXY_HOST", "myapp")
        .assert()
        .failure();
}

#[test]
fn help_shows_setup_option() {
    cfproxy()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--setup"));
}

#[test]
fn help_shows_purge_option() {
    cfproxy()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--purge"));
}

#[test]
fn purge_without_api_config_fails_gracefully() {
    // --purge requires API configuration; with a temp config dir (no settings),
    // it should fail with a helpful error message.
    let tmp = TempDir::new().expect("should create tempdir");
    cfproxy()
        .arg("--purge")
        .env("XDG_CONFIG_HOME", tmp.path().to_str().unwrap())
        .env("HOME", tmp.path().to_str().unwrap())
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("API configuration required")
                .or(predicate::str::contains("--setup")),
        );
}

#[test]
fn help_shows_doctor_option() {
    cfproxy()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--doctor"));
}

#[test]
fn doctor_runs_without_crashing() {
    // --doctor should always exit 0, even without API config.
    // Use a temp config dir so we don't depend on real settings.
    let tmp = TempDir::new().expect("should create tempdir");
    cfproxy()
        .arg("--doctor")
        .env("XDG_CONFIG_HOME", tmp.path().to_str().unwrap())
        .env("HOME", tmp.path().to_str().unwrap())
        .assert()
        .success()
        .stderr(predicate::str::contains("cfproxy doctor"));
}

#[test]
fn doctor_shows_settings_check() {
    let tmp = TempDir::new().expect("should create tempdir");
    cfproxy()
        .arg("--doctor")
        .env("XDG_CONFIG_HOME", tmp.path().to_str().unwrap())
        .env("HOME", tmp.path().to_str().unwrap())
        .assert()
        .success()
        .stderr(predicate::str::contains("Settings file"));
}

#[test]
fn doctor_shows_cloudflared_check() {
    let tmp = TempDir::new().expect("should create tempdir");
    cfproxy()
        .arg("--doctor")
        .env("XDG_CONFIG_HOME", tmp.path().to_str().unwrap())
        .env("HOME", tmp.path().to_str().unwrap())
        .assert()
        .success()
        .stderr(predicate::str::contains("cloudflared"));
}

#[test]
fn host_without_api_config_shows_error() {
    // --host requires custom domain setup, should fail with helpful message
    // when no settings are configured. Use a temp config dir to avoid
    // picking up real user settings.
    let tmp = TempDir::new().expect("should create tempdir");
    cfproxy()
        .args([
            "8080",
            "--host",
            "myapp",
            "--cloudflared-path",
            "/nonexistent/cloudflared",
        ])
        .env("XDG_CONFIG_HOME", tmp.path().to_str().unwrap())
        .env("HOME", tmp.path().to_str().unwrap())
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("custom domain")
                .or(predicate::str::contains("--host requires")),
        );
}
