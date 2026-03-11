use regex::Regex;
use tempfile::tempdir;

use zerox0_ai::report::build_writeup;
use zerox0_ai::storage::{ArtifactRecord, NewAction, StateStore};

#[test]
fn writeup_matches_golden_fixture() {
    let tmp = tempdir().expect("tmp");
    let db = tmp.path().join("state.db");
    let store = StateStore::open(&db, 100, 100, 100).expect("store");

    store
        .create_session("sess1", "/tmp/ctf")
        .expect("create session");
    store
        .touch_session(
            "sess1",
            Some("active"),
            Some("crypto"),
            Some("summary text"),
        )
        .expect("touch");

    store
        .upsert_artifact(&ArtifactRecord {
            session_id: "sess1".to_string(),
            path: "/tmp/ctf/challenge.txt".to_string(),
            kind: "text".to_string(),
            size: 42,
            sha256: Some("abcd".to_string()),
            mime: Some("text/plain".to_string()),
            indexed_at: "2026-01-01T00:00:00Z".to_string(),
            summary: Some("rsa params".to_string()),
        })
        .expect("artifact");

    store
        .add_hypothesis("sess1", "Try common modulus attack", 0.7, "open")
        .expect("hypothesis");
    store
        .add_note("sess1", "Need factorization sanity check")
        .expect("note");
    store
        .add_citation(
            "sess1",
            "web",
            "https://example.com",
            Some("line:1"),
            "rsa background",
        )
        .expect("citation");

    store
        .add_action(NewAction {
            session_id: "sess1",
            action_type: "triage",
            command: "file /tmp/ctf/challenge.txt",
            target: Some("/tmp/ctf/challenge.txt"),
            status: "ok",
            stdout: Some("FLAG{fixture_report}"),
            stderr: Some(""),
            metadata: None,
        })
        .expect("action");

    let bundle = build_writeup(&store, "sess1").expect("writeup");
    let normalized = normalize_markdown(&bundle.markdown);
    let expected = include_str!("fixtures/reports/expected_writeup.md");
    assert_eq!(normalized, expected);
}

fn normalize_markdown(input: &str) -> String {
    let ts = Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?\+?\d{0,2}:?\d{0,2}Z?")
        .expect("timestamp regex");
    ts.replace_all(input, "<TS>").to_string()
}
