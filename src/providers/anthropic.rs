use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};

use crate::config::AnthropicProvider;
use crate::util::{chunk_text, estimate_tokens};

use super::{Provider, ProviderRequest, ProviderResponse};

pub struct AnthropicProviderClient {
    name: String,
    api_key: String,
    base_url: String,
    default_model: String,
}

impl AnthropicProviderClient {
    pub fn from_config(cfg: &AnthropicProvider) -> Option<Self> {
        if !cfg.enabled {
            return None;
        }
        let key = cfg
            .api_key
            .clone()
            .or_else(|| std::env::var(&cfg.api_key_env).ok())?;
        if key.trim().is_empty() {
            return None;
        }

        Some(Self {
            name: cfg.name.clone(),
            api_key: key,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            default_model: cfg.default_model.clone(),
        })
    }
}

impl Provider for AnthropicProviderClient {
    fn name(&self) -> &str {
        &self.name
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn list_models(&self, timeout_secs: u64) -> Result<Vec<String>> {
        let endpoint = format!("{}/models", self.base_url);
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&self.api_key).context("invalid anthropic api key header")?,
        );
        headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .context("building anthropic client")?;

        let response = client
            .get(endpoint)
            .headers(headers)
            .send()
            .context("requesting anthropic model list")?;

        if !response.status().is_success() {
            bail!("anthropic model list returned HTTP {}", response.status());
        }

        let value: Value = response.json().context("parsing anthropic model list")?;
        let mut models = value
            .get("data")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.get("id").and_then(Value::as_str))
                    .map(ToString::to_string)
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

        if let Some(sink) = stream.as_mut()
            && let Ok(text) = self.generate_streaming(req, &model, *sink)
        {
            return Ok(ProviderResponse {
                provider: self.name.clone(),
                model,
                prompt_tokens_est: estimate_tokens(&req.prompt),
                completion_tokens_est: estimate_tokens(&text),
                text,
            });
        }

        let endpoint = format!("{}/messages", self.base_url);

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&self.api_key).context("invalid anthropic api key header")?,
        );
        headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let body = json!({
            "model": model,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "system": req.system,
            "messages": [
                {"role": "user", "content": req.prompt}
            ]
        });

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(req.timeout_secs))
            .build()
            .context("building anthropic client")?;

        let response = client
            .post(endpoint)
            .headers(headers)
            .json(&body)
            .send()
            .context("sending anthropic request")?;

        if !response.status().is_success() {
            bail!(
                "anthropic-compatible provider returned HTTP {}",
                response.status()
            );
        }

        let value: Value = response.json().context("parsing anthropic response")?;
        let text = extract_text(&value)?;

        if let Some(sink) = stream.as_mut() {
            for c in chunk_text(&text, 52) {
                sink(&c);
            }
        }

        Ok(ProviderResponse {
            provider: self.name.clone(),
            model,
            prompt_tokens_est: estimate_tokens(&req.prompt),
            completion_tokens_est: estimate_tokens(&text),
            text,
        })
    }
}

impl AnthropicProviderClient {
    fn generate_streaming(
        &self,
        req: &ProviderRequest,
        model: &str,
        sink: &mut dyn FnMut(&str),
    ) -> Result<String> {
        let endpoint = format!("{}/messages", self.base_url);
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&self.api_key).context("invalid anthropic api key header")?,
        );
        headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let body = json!({
            "model": model,
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "system": req.system,
            "stream": true,
            "messages": [
                {"role": "user", "content": req.prompt}
            ]
        });

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(req.timeout_secs))
            .build()
            .context("building anthropic client")?;

        let response = client
            .post(endpoint)
            .headers(headers)
            .json(&body)
            .send()
            .context("sending anthropic streaming request")?;

        if !response.status().is_success() {
            bail!(
                "anthropic-compatible streaming returned HTTP {}",
                response.status()
            );
        }

        let mut reader = BufReader::new(response);
        let mut line = String::new();
        let mut text = String::new();

        loop {
            line.clear();
            let n = reader.read_line(&mut line).context("reading stream line")?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim();
            if !trimmed.starts_with("data:") {
                continue;
            }
            let data = trimmed.trim_start_matches("data:").trim();
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                break;
            }
            if let Ok(value) = serde_json::from_str::<Value>(data)
                && let Some(chunk) = extract_stream_text(&value)
            {
                sink(&chunk);
                text.push_str(&chunk);
            }
        }

        if text.trim().is_empty() {
            bail!("stream returned empty text");
        }
        Ok(text)
    }
}

fn extract_text(value: &Value) -> Result<String> {
    let content = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("missing content array"))?;

    let text = content
        .iter()
        .filter_map(|c| c.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");

    if text.is_empty() {
        bail!("empty anthropic text")
    } else {
        Ok(text)
    }
}

fn extract_stream_text(value: &Value) -> Option<String> {
    if let Some(t) = value
        .get("delta")
        .and_then(|d| d.get("text"))
        .and_then(Value::as_str)
    {
        return Some(t.to_string());
    }
    if let Some(t) = value
        .get("content_block")
        .and_then(|d| d.get("text"))
        .and_then(Value::as_str)
    {
        return Some(t.to_string());
    }
    value
        .get("content")
        .and_then(Value::as_array)
        .and_then(|arr| {
            let joined = arr
                .iter()
                .filter_map(|c| c.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        })
}
