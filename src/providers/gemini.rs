use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde_json::{Value, json};

use crate::config::GeminiProvider;
use crate::util::{chunk_text, estimate_tokens};

use super::{Provider, ProviderRequest, ProviderResponse};

pub struct GeminiProviderClient {
    api_key: String,
    base_url: String,
    default_model: String,
}

impl GeminiProviderClient {
    pub fn from_config(cfg: &GeminiProvider) -> Option<Self> {
        if !cfg.enabled {
            return None;
        }
        let api_key = cfg
            .api_key
            .clone()
            .or_else(|| std::env::var(&cfg.api_key_env).ok())?;
        if api_key.trim().is_empty() {
            return None;
        }
        Some(Self {
            api_key,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            default_model: cfg.default_model.clone(),
        })
    }
}

impl Provider for GeminiProviderClient {
    fn name(&self) -> &str {
        "gemini"
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn list_models(&self, timeout_secs: u64) -> Result<Vec<String>> {
        let endpoint = format!("{}/models?key={}", self.base_url, self.api_key);
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .context("building gemini client")?;

        let response = client
            .get(endpoint)
            .send()
            .context("requesting gemini model list")?;

        if !response.status().is_success() {
            bail!("gemini model list returned HTTP {}", response.status());
        }

        let value: Value = response.json().context("parsing gemini model list")?;
        let mut models = value
            .get("models")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.get("name").and_then(Value::as_str))
                    .map(|name| name.trim_start_matches("models/").to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if models.is_empty() {
            models.push(self.default_model.clone());
        }
        Ok(models)
    }

    fn generate(
        &self,
        req: &ProviderRequest,
        mut stream: Option<&mut dyn FnMut(&str)>,
    ) -> Result<ProviderResponse> {
        let model = req
            .model_override
            .clone()
            .unwrap_or_else(|| self.default_model.clone());

        let endpoint = format!(
            "{}/models/{}:generateContent?key={}",
            self.base_url, model, self.api_key
        );

        let prompt = if let Some(system) = &req.system {
            format!("System:\n{}\n\nUser:\n{}", system, req.prompt)
        } else {
            req.prompt.clone()
        };

        let body = json!({
            "contents": [
                { "parts": [ { "text": prompt } ] }
            ],
            "generationConfig": {
                "temperature": req.temperature,
                "maxOutputTokens": req.max_tokens
            }
        });

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(req.timeout_secs))
            .build()
            .context("building gemini client")?;

        let response = client
            .post(endpoint)
            .json(&body)
            .send()
            .context("sending gemini request")?;

        if !response.status().is_success() {
            bail!("gemini returned HTTP {}", response.status().as_u16());
        }

        let value: Value = response.json().context("parsing gemini json")?;
        let text = extract_text(&value)?;

        if let Some(sink) = stream.as_mut() {
            for c in chunk_text(&text, 52) {
                sink(&c);
            }
        }

        Ok(ProviderResponse {
            provider: "gemini".to_string(),
            model,
            prompt_tokens_est: estimate_tokens(&req.prompt),
            completion_tokens_est: estimate_tokens(&text),
            text,
        })
    }
}

fn extract_text(value: &Value) -> Result<String> {
    let candidates = value
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("missing candidates"))?;
    let first = candidates
        .first()
        .ok_or_else(|| anyhow::anyhow!("empty candidates"))?;
    let parts = first
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("missing content.parts"))?;

    let text = parts
        .iter()
        .filter_map(|p| p.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");

    if text.is_empty() {
        bail!("empty text in gemini response")
    } else {
        Ok(text)
    }
}
