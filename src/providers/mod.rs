mod anthropic;
mod gemini;
mod generic;
mod local;
mod openai_compat;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use crate::config::{AppConfig, TaskRoute};
use crate::util::{chunk_text, estimate_tokens};

pub use local::LocalProvider;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TaskType {
    Reasoning,
    Coding,
    Summarization,
    Vision,
    Classification,
}

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub system: Option<String>,
    pub prompt: String,
    pub task_type: TaskType,
    pub max_tokens: usize,
    pub temperature: f32,
    pub timeout_secs: u64,
    pub model_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResponse {
    pub provider: String,
    pub model: String,
    pub text: String,
    pub prompt_tokens_est: usize,
    pub completion_tokens_est: usize,
}

pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn default_model(&self) -> &str;
    fn list_models(&self, _timeout_secs: u64) -> Result<Vec<String>> {
        Ok(vec![self.default_model().to_string()])
    }
    fn generate(
        &self,
        req: &ProviderRequest,
        stream: Option<&mut dyn FnMut(&str)>,
    ) -> Result<ProviderResponse>;
}

pub struct ProviderManager {
    providers: HashMap<String, Box<dyn Provider>>,
    config: AppConfig,
}

impl ProviderManager {
    pub fn new(config: AppConfig) -> Self {
        let mut providers: HashMap<String, Box<dyn Provider>> = HashMap::new();

        let local = local::LocalProvider::new();
        providers.insert(local.name().to_string(), Box::new(local));

        if let Some(p) =
            openai_compat::OpenAiCompatProvider::from_named("openai", &config.providers.openai)
        {
            providers.insert("openai".to_string(), Box::new(p));
        }
        if let Some(p) = openai_compat::OpenAiCompatProvider::from_named(
            "openrouter",
            &config.providers.openrouter,
        ) {
            providers.insert("openrouter".to_string(), Box::new(p));
        }
        if let Some(p) =
            openai_compat::OpenAiCompatProvider::from_named("together", &config.providers.together)
        {
            providers.insert("together".to_string(), Box::new(p));
        }
        if let Some(p) =
            openai_compat::OpenAiCompatProvider::from_named("moonshot", &config.providers.moonshot)
        {
            providers.insert("moonshot".to_string(), Box::new(p));
        }

        if let Some(p) =
            anthropic::AnthropicProviderClient::from_config(&config.providers.anthropic)
        {
            providers.insert("anthropic".to_string(), Box::new(p));
        }

        for cfg in &config.providers.anthropic_compatible {
            if let Some(p) = anthropic::AnthropicProviderClient::from_config(cfg) {
                providers.insert(cfg.name.clone(), Box::new(p));
            }
        }

        if let Some(p) = gemini::GeminiProviderClient::from_config(&config.providers.gemini) {
            providers.insert("gemini".to_string(), Box::new(p));
        }

        for cfg in &config.providers.custom_openai_compatible {
            if let Some(p) = openai_compat::OpenAiCompatProvider::from_custom(cfg) {
                providers.insert(cfg.name.clone(), Box::new(p));
            }
        }

        for cfg in &config.providers.generic_http {
            if let Some(p) = generic::GenericHttpProviderClient::from_config(cfg) {
                providers.insert(cfg.name.clone(), Box::new(p));
            }
        }

        Self { providers, config }
    }

    pub fn available_provider_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.providers.keys().cloned().collect();
        names.sort_unstable();
        names
    }

    pub fn call(
        &self,
        request: ProviderRequest,
        stream: Option<&mut dyn FnMut(&str)>,
    ) -> Result<ProviderResponse> {
        let route = self.route_for(request.task_type);
        let provider_name = route
            .map(|r| r.provider.clone())
            .unwrap_or_else(|| "local".to_string());
        let response = self.call_with_provider(&provider_name, request, stream)?;
        if let Some(route) = route
            && let Some(model_override) = &route.model
        {
            return Ok(ProviderResponse {
                model: model_override.clone(),
                ..response
            });
        }
        Ok(response)
    }

    pub fn call_with_provider(
        &self,
        provider_name: &str,
        request: ProviderRequest,
        stream: Option<&mut dyn FnMut(&str)>,
    ) -> Result<ProviderResponse> {
        let provider = self
            .providers
            .get(provider_name)
            .or_else(|| self.providers.get("local"))
            .ok_or_else(|| anyhow::anyhow!("no provider available"))?;

        let prompt_tokens = estimate_tokens(&request.prompt)
            + request
                .system
                .as_deref()
                .map(estimate_tokens)
                .unwrap_or_default();

        if prompt_tokens > self.config.providers.max_input_tokens {
            bail!(
                "prompt token budget exceeded: {prompt_tokens} > {}",
                self.config.providers.max_input_tokens
            )
        }

        if stream.is_some() {
            return provider.generate(&request, stream);
        }

        let retries = self.config.providers.retries;
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 0..retries {
            match provider.generate(&request, None) {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    last_err = Some(err);
                    let backoff = self.config.providers.backoff_base_ms * (2_u64.pow(attempt));
                    thread::sleep(Duration::from_millis(backoff));
                }
            }
        }
        match provider.generate(&request, None) {
            Ok(resp) => Ok(resp),
            Err(err) => Err(last_err.unwrap_or(err)),
        }
    }

    pub fn stream_text_chunks(text: &str, mut stream: Option<&mut dyn FnMut(&str)>) {
        if let Some(sink) = stream.as_mut() {
            for chunk in chunk_text(text, 48) {
                sink(&chunk);
            }
        }
    }

    pub fn provider_for_task(&self, task: TaskType) -> String {
        self.route_for(task)
            .map(|r| r.provider.clone())
            .unwrap_or_else(|| "local".to_string())
    }

    pub fn list_models(&self, provider: Option<&str>) -> Result<BTreeMap<String, Vec<String>>> {
        let timeout = self.config.providers.request_timeout_secs;
        let mut out = BTreeMap::new();
        if let Some(name) = provider {
            if let Some(p) = self.providers.get(name) {
                let mut models = p.list_models(timeout)?;
                models.sort();
                models.dedup();
                out.insert(name.to_string(), models);
            } else {
                bail!("provider '{}' is not available", name);
            }
            return Ok(out);
        }

        for (name, provider) in &self.providers {
            let mut models = provider
                .list_models(timeout)
                .unwrap_or_else(|_| vec![provider.default_model().to_string()]);
            models.sort();
            models.dedup();
            out.insert(name.clone(), models);
        }

        Ok(out)
    }

    fn route_for(&self, task: TaskType) -> Option<&TaskRoute> {
        match task {
            TaskType::Reasoning => Some(&self.config.model_routing.reasoning),
            TaskType::Coding => Some(&self.config.model_routing.coding),
            TaskType::Summarization => Some(&self.config.model_routing.summarization),
            TaskType::Vision => Some(&self.config.model_routing.vision),
            TaskType::Classification => Some(&self.config.model_routing.classification),
        }
    }
}
