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
