use std::path::PathBuf;

use zerox0_ai::config::SafetyConfig;
use zerox0_ai::policy::{Approvals, PolicyEngine};

#[test]
fn blocks_unapproved_exec() {
    let cfg = SafetyConfig {
        require_confirmation_for_exec: true,
        ..SafetyConfig::default()
    };
    let policy = PolicyEngine::new(cfg).expect("policy");
    let err = policy
        .ensure_exec_allowed(Approvals::default(), "curl")
        .expect_err("expected block");
    assert!(err.to_string().contains("blocked"));
}

#[test]
fn blocks_non_allowlisted_target() {
    let cfg = SafetyConfig {
        allowed_hosts: vec!["127.0.0.1".to_string()],
        allowed_ports: vec![8000],
        ..SafetyConfig::default()
    };
    let policy = PolicyEngine::new(cfg).expect("policy");

    let approvals = Approvals {
        network: true,
        exec: true,
        install: false,
    };

    let err = policy
        .ensure_network_allowed(approvals, "example.com", Some(80), false)
        .expect_err("expected block");
    assert!(err.to_string().contains("not in safety.allowed_hosts"));
}

#[test]
fn allows_path_in_allowlist() {
    let cfg = SafetyConfig {
        allowed_paths: vec![PathBuf::from("tests")],
        ..SafetyConfig::default()
    };
    let policy = PolicyEngine::new(cfg).expect("policy");
    policy
        .ensure_path_allowed(PathBuf::from("tests/fixtures").as_path())
        .expect("allowed");
}
