use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn help_lists_core_commands() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("0x0"));
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("scan"))
        .stdout(predicate::str::contains("solve"))
        .stdout(predicate::str::contains("research"))
        .stdout(predicate::str::contains("web"));
}

#[test]
fn init_and_scan_work_in_temp_profile() {
    let sandbox = TempDir::new().expect("tmp");
    let xdg = sandbox.path().join("xdg");
    std::fs::create_dir_all(&xdg).expect("xdg");

    let config = sandbox.path().join("config.toml");

    let mut init = Command::new(assert_cmd::cargo::cargo_bin!("0x0"));
    init.arg("--config")
        .arg(&config)
        .arg("init")
        .arg(sandbox.path())
        .env("XDG_CONFIG_HOME", &xdg)
        .env("XDG_DATA_HOME", &xdg)
        .env("XDG_CACHE_HOME", &xdg)
        .assert()
        .success();

    let mut scan = Command::new(assert_cmd::cargo::cargo_bin!("0x0"));
    scan.arg("--config")
        .arg(&config)
        .arg("scan")
        .arg("tests/fixtures/misc")
        .arg("--json")
        .env("XDG_CONFIG_HOME", &xdg)
        .env("XDG_DATA_HOME", &xdg)
        .env("XDG_CACHE_HOME", &xdg)
        .assert()
        .success()
        .stdout(predicate::str::contains("detected_category"));
}

#[test]
fn sessions_command_lists_and_filters() {
    let sandbox = TempDir::new().expect("tmp");
    let xdg = sandbox.path().join("xdg");
    std::fs::create_dir_all(&xdg).expect("xdg");
    let config = sandbox.path().join("config.toml");

    let mut init = Command::new(assert_cmd::cargo::cargo_bin!("0x0"));
    init.arg("--config")
        .arg(&config)
        .arg("init")
        .arg(sandbox.path())
        .env("XDG_CONFIG_HOME", &xdg)
        .env("XDG_DATA_HOME", &xdg)
        .env("XDG_CACHE_HOME", &xdg)
        .assert()
        .success();

    let mut scan_misc = Command::new(assert_cmd::cargo::cargo_bin!("0x0"));
    scan_misc
        .arg("--config")
        .arg(&config)
        .arg("scan")
        .arg("tests/fixtures/misc")
        .arg("--session-id")
        .arg("sess-misc")
        .env("XDG_CONFIG_HOME", &xdg)
        .env("XDG_DATA_HOME", &xdg)
        .env("XDG_CACHE_HOME", &xdg)
        .assert()
        .success();

    let mut scan_crypto = Command::new(assert_cmd::cargo::cargo_bin!("0x0"));
    scan_crypto
        .arg("--config")
        .arg(&config)
        .arg("scan")
        .arg("tests/fixtures/crypto")
        .arg("--session-id")
        .arg("sess-crypto")
        .env("XDG_CONFIG_HOME", &xdg)
        .env("XDG_DATA_HOME", &xdg)
        .env("XDG_CACHE_HOME", &xdg)
        .assert()
        .success();

    let mut sessions = Command::new(assert_cmd::cargo::cargo_bin!("0x0"));
    sessions
        .arg("--config")
        .arg(&config)
        .arg("sessions")
        .arg("--limit")
        .arg("10")
        .arg("--json")
        .env("XDG_CONFIG_HOME", &xdg)
        .env("XDG_DATA_HOME", &xdg)
        .env("XDG_CACHE_HOME", &xdg)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"sessions\""))
        .stdout(predicate::str::contains("\"id\": \"sess-misc\""))
        .stdout(predicate::str::contains("\"id\": \"sess-crypto\""))
        .stdout(predicate::str::contains("\"action_count\""));

    let mut filtered = Command::new(assert_cmd::cargo::cargo_bin!("0x0"));
    filtered
        .arg("--config")
        .arg(&config)
        .arg("sessions")
        .arg("--category")
        .arg("misc")
        .arg("--json")
        .env("XDG_CONFIG_HOME", &xdg)
        .env("XDG_DATA_HOME", &xdg)
        .env("XDG_CACHE_HOME", &xdg)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"id\": \"sess-misc\""))
        .stdout(predicate::str::contains("\"id\": \"sess-crypto\"").not());
}
