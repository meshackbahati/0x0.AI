use httpmock::Method::POST;
use httpmock::MockServer;
use serde_json::json;

use zerox0_ai::config::AppConfig;
use zerox0_ai::providers::{ProviderManager, ProviderRequest, TaskType};

#[test]
fn openai_compat_provider_works_with_mock_server() {
    let server = MockServer::start();

    let _mock = server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "choices": [
                    {"message": {"content": "mock completion"}}
                ]
            }));
    });

    let mut cfg = AppConfig::default();
    cfg.providers.openai.enabled = true;
    cfg.providers.openai.base_url = server.base_url();
    cfg.model_routing.reasoning.provider = "openai".to_string();

    unsafe { std::env::set_var("OPENAI_API_KEY", "test-key") };

    let manager = ProviderManager::new(cfg);
    let response = manager
        .call(
            ProviderRequest {
                system: None,
                prompt: "hello".to_string(),
                task_type: TaskType::Reasoning,
                max_tokens: 32,
                temperature: 0.0,
                timeout_secs: 5,
                model_override: None,
            },
            None,
        )
        .expect("provider response");

    assert_eq!(response.text, "mock completion");
}
