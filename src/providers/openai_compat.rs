use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};

use crate::config::NamedOpenAiCompatProvider;
use crate::config::OpenAiCompatProvider as OpenAiCompatProviderConfig;
use crate::util::{chunk_text, estimate_tokens};

use super::{Provider, ProviderRequest, ProviderResponse};

pub struct OpenAiCompatProvider {
    name: String,
    base_url: String,
    api_key: String,
    default_model: String,
}

impl OpenAiCompatProvider {
    pub fn from_named(name: &str, cfg: &OpenAiCompatProviderConfig) -> Option<Self> {
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
            name: name.to_string(),
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            api_key,
            default_model: cfg.default_model.clone(),
        })
    }

    pub fn from_custom(cfg: &NamedOpenAiCompatProvider) -> Option<Self> {
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
            name: cfg.name.clone(),
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            api_key,
            default_model: cfg.default_model.clone(),
        })
    }
}

impl Provider for OpenAiCompatProvider {
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
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                .context("building authorization header")?,
        );

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .context("building http client")?;

        let response = client
            .get(endpoint)
            .headers(headers)
            .send()
            .context("requesting model list")?;

        if !response.status().is_success() {
            bail!(
                "provider {} model list returned HTTP {}",
                self.name,
                response.status()
            );
        }

        let value: Value = response.json().context("parsing model list response")?;
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

        let endpoint = format!("{}/chat/completions", self.base_url);
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                .context("building authorization header")?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(req.timeout_secs))
            .build()
            .context("building http client")?;

        let mut messages = Vec::new();
        if let Some(system) = &req.system {
            messages.push(json!({"role": "system", "content": system}));
        }
        messages.push(json!({"role": "user", "content": req.prompt}));

        let body = json!({
            "model": model,
            "messages": messages,
            "temperature": req.temperature,
            "max_tokens": req.max_tokens,
            "stream": false
        });

        let response = client
            .post(endpoint)
            .headers(headers)
            .json(&body)
            .send()
            .context("sending provider request")?;

        if !response.status().is_success() {
            bail!(
                "provider {} returned HTTP {}",
                self.name,
                response.status().as_u16()
            );
        }

        let value: Value = response.json().context("decoding provider response json")?;
        let text = extract_text_from_openai_compat(&value)
            .context("extracting completion text from provider response")?;

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

impl OpenAiCompatProvider {
    fn generate_streaming(
        &self,
        req: &ProviderRequest,
        model: &str,
        sink: &mut dyn FnMut(&str),
    ) -> Result<String> {
        let endpoint = format!("{}/chat/completions", self.base_url);

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                .context("building authorization header")?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(req.timeout_secs))
            .build()
            .context("building http client")?;

        let mut messages = Vec::new();
        if let Some(system) = &req.system {
            messages.push(json!({"role": "system", "content": system}));
        }
        messages.push(json!({"role": "user", "content": req.prompt}));

        let body = json!({
            "model": model,
            "messages": messages,
            "temperature": req.temperature,
            "max_tokens": req.max_tokens,
            "stream": true
        });

        let response = client
            .post(endpoint)
            .headers(headers)
            .json(&body)
            .send()
            .context("sending streaming provider request")?;

        if !response.status().is_success() {
            bail!(
                "provider {} streaming returned HTTP {}",
                self.name,
                response.status().as_u16()
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

            if let Ok(value) = serde_json::from_str::<Value>(data) {
                if let Some(chunk) = extract_openai_stream_chunk(&value) {
                    sink(&chunk);
                    text.push_str(&chunk);
                }
                continue;
            }

            sink(data);
            text.push_str(data);
        }

        if text.trim().is_empty() {
            bail!("stream returned empty text");
        }
        Ok(text)
    }
}

fn extract_text_from_openai_compat(value: &Value) -> Result<String> {
    let Some(choice) = value.get("choices").and_then(|c| c.get(0)) else {
        bail!("missing choices[0] in response")
    };

    if let Some(text) = choice.get("message").and_then(|m| m.get("content")) {
        if let Some(s) = text.as_str() {
            return Ok(s.to_string());
        }
        if text.is_array() {
            let joined = text
                .as_array()
                .unwrap_or(&Vec::new())
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            if !joined.is_empty() {
                return Ok(joined);
            }
        }
    }

    if let Some(text) = choice.get("text").and_then(Value::as_str) {
        return Ok(text.to_string());
    }

    bail!("could not extract text from response")
}

fn extract_openai_stream_chunk(value: &Value) -> Option<String> {
    let choice = value.get("choices")?.get(0)?;

    if let Some(content) = choice
        .get("delta")
        .and_then(|d| d.get("content"))
        .and_then(Value::as_str)
    {
        return Some(content.to_string());
    }

    if let Some(text) = choice
        .get("delta")
        .and_then(|d| d.get("text"))
        .and_then(Value::as_str)
    {
        return Some(text.to_string());
    }

    if let Some(content) = choice
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
    {
        return Some(content.to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text() {
        let value = json!({"choices": [{"message": {"content": "hello"}}]});
        let text = extract_text_from_openai_compat(&value).expect("text");
        assert_eq!(text, "hello");
    }
}
