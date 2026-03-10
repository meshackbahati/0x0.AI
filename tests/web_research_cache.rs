use httpmock::Method::GET;
use httpmock::MockServer;
use tempfile::tempdir;

use zerox0_ai::config::ResearchConfig;
use zerox0_ai::research::web::WebResearcher;
use zerox0_ai::storage::StateStore;

#[test]
fn fetch_url_is_cached() {
    let server = MockServer::start();

    let _mock_page = server.mock(|when, then| {
        when.method(GET).path("/");
        then.status(200)
            .header("content-type", "text/html")
            .body("<html><head><title>Fixture</title></head><body><p>Hello test cache</p></body></html>");
    });

    let _mock_robots = server.mock(|when, then| {
        when.method(GET).path("/robots.txt");
        then.status(200).body("User-agent: *\nDisallow:\n");
    });

    let tmp = tempdir().expect("tmp");
    let db = tmp.path().join("state.db");
    let store = StateStore::open(&db, 100, 100, 100).expect("store");

    let cfg = ResearchConfig {
        respect_robots: true,
        ..ResearchConfig::default()
    };
    let researcher = WebResearcher::new(cfg).expect("researcher");

    let url = format!("{}{}", server.base_url(), "/");
    let first = researcher
        .fetch_url(&url, &store)
        .expect("fetch")
        .expect("page");
    assert!(!first.from_cache);

    let second = researcher
        .fetch_url(&url, &store)
        .expect("fetch")
        .expect("page");
    assert!(second.from_cache);
}
