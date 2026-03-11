use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};

use crate::config::GenericHttpProvider;
use crate::util::{chunk_text, estimate_tokens};

use super::{Provider, ProviderRequest, ProviderResponse};

pub struct GenericHttpProviderClient {
    name: String,
    base_url: String,
    api_key: String,
    default_model: String,
}

impl GenericHttpProviderClient {
    pub fn from_config(cfg: &GenericHttpProvider) -> Option<Self> {
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

impl Provider for GenericHttpProviderClient {
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
            .context("building generic provider client")?;

        let response = client.get(endpoint).headers(headers).send();
        match response {
            Ok(resp) if resp.status().is_success() => {
                let value: Value = resp.json().context("parsing model list")?;
                let mut models = extract_models(&value);
                if models.is_empty() {
                    models.push(self.default_model.clone());
                }
                Ok(models)
            }
            _ => Ok(vec![self.default_model.clone()]),
        }
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

        let endpoint = self.base_url.clone();
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                .context("building authorization header")?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let body = generic_body(req, &model, false);

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(req.timeout_secs))
            .build()
            .context("building generic provider client")?;

        let response = client
            .post(endpoint)
            .headers(headers)
            .json(&body)
            .send()
            .context("sending generic provider request")?;

        if !response.status().is_success() {
            bail!(
                "generic provider {} returned HTTP {}",
                self.name,
                response.status().as_u16()
            );
        }

        let text =
            decode_generic_response(response).context("decoding generic provider response")?;

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

impl GenericHttpProviderClient {
    fn generate_streaming(
        &self,
        req: &ProviderRequest,
        model: &str,
        sink: &mut dyn FnMut(&str),
    ) -> Result<String> {
        let endpoint = self.base_url.clone();
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                .context("building authorization header")?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let body = generic_body(req, model, true);

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(req.timeout_secs))
            .build()
            .context("building generic provider client")?;

        let response = client
            .post(endpoint)
            .headers(headers)
            .json(&body)
            .send()
            .context("sending generic streaming request")?;

        if !response.status().is_success() {
            bail!(
                "generic provider {} streaming returned HTTP {}",
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
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(data)
                && let Some(chunk) = extract_generic_text(&value)
            {
                sink(&chunk);
                text.push_str(&chunk);
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

fn generic_body(req: &ProviderRequest, model: &str, stream: bool) -> Value {
    let mut messages = Vec::new();
    if let Some(system) = &req.system {
        messages.push(json!({"role": "system", "content": system}));
    }
    messages.push(json!({"role": "user", "content": req.prompt}));

    json!({
        "model": model,
        "prompt": req.prompt,
        "input": req.prompt,
        "system": req.system,
        "messages": messages,
        "max_tokens": req.max_tokens,
        "temperature": req.temperature,
        "stream": stream
    })
}

fn decode_generic_response(response: reqwest::blocking::Response) -> Result<String> {
    let text_body = response.text().context("reading generic response body")?;
    if let Ok(value) = serde_json::from_str::<Value>(&text_body)
        && let Some(text) = extract_generic_text(&value)
    {
        return Ok(text);
    }

    let trimmed = text_body.trim();
    if trimmed.is_empty() {
        bail!("generic provider returned empty response")
    } else {
        Ok(trimmed.to_string())
    }
}

fn extract_models(value: &Value) -> Vec<String> {
    if let Some(arr) = value.get("data").and_then(Value::as_array) {
        return arr
            .iter()
            .filter_map(|v| {
                v.get("id")
                    .or_else(|| v.get("name"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .collect();
    }
    if let Some(arr) = value.get("models").and_then(Value::as_array) {
        return arr
            .iter()
            .filter_map(|v| {
                v.get("id")
                    .or_else(|| v.get("name"))
                    .or_else(|| v.get("model"))
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .collect();
    }
    Vec::new()
}

fn extract_generic_text(value: &Value) -> Option<String> {
    let direct_keys = [
        "text",
        "output_text",
        "output",
        "response",
        "answer",
        "message",
    ];
    for key in direct_keys {
        if let Some(v) = value.get(key).and_then(Value::as_str)
            && !v.trim().is_empty()
        {
            return Some(v.to_string());
        }
    }

    if let Some(v) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
    {
        return Some(v.to_string());
    }

    if let Some(v) = value
        .get("delta")
        .and_then(|d| d.get("content"))
        .and_then(Value::as_str)
    {
        return Some(v.to_string());
    }

    if let Some(v) = value.get("content").and_then(Value::as_array).map(|arr| {
        arr.iter()
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    }) && !v.trim().is_empty()
    {
        return Some(v);
    }

    if let Some(v) = value
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        && !v.trim().is_empty()
    {
        return Some(v);
    }

    None
}
