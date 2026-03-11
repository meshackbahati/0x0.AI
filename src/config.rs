use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub safety: SafetyConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub model_routing: ModelRouting,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub research: ResearchConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub workspace_root: Option<PathBuf>,
    pub default_json_output: bool,
    pub log_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyConfig {
    pub allowed_paths: Vec<PathBuf>,
    pub allowed_hosts: Vec<String>,
    pub allowed_ports: Vec<u16>,
    pub offline_only: bool,
    pub research_web_enabled: bool,
    pub require_confirmation_for_network: bool,
    pub require_confirmation_for_exec: bool,
    pub require_confirmation_for_install: bool,
    pub max_runtime_per_action_secs: u64,
    pub max_parallel_actions: usize,
    pub redact_secrets_in_logs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvidersConfig {
    pub fallback_local: bool,
    pub request_timeout_secs: u64,
    pub retries: u32,
    pub backoff_base_ms: u64,
    pub max_input_tokens: usize,
    pub openai: OpenAiCompatProvider,
    pub openrouter: OpenAiCompatProvider,
    pub together: OpenAiCompatProvider,
    pub moonshot: OpenAiCompatProvider,
    pub anthropic: AnthropicProvider,
    pub gemini: GeminiProvider,
    pub anthropic_compatible: Vec<AnthropicProvider>,
    pub custom_openai_compatible: Vec<NamedOpenAiCompatProvider>,
    #[serde(default)]
    pub generic_http: Vec<GenericHttpProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiCompatProvider {
    pub enabled: bool,
    pub base_url: String,
    pub api_key_env: String,
    pub api_key: Option<String>,
    pub default_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicProvider {
    pub name: String,
    pub enabled: bool,
    pub base_url: String,
    pub api_key_env: String,
    pub api_key: Option<String>,
    pub default_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiProvider {
    pub enabled: bool,
    pub api_key_env: String,
    pub api_key: Option<String>,
    pub base_url: String,
    pub default_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedOpenAiCompatProvider {
    pub name: String,
    pub enabled: bool,
    pub base_url: String,
    pub api_key_env: String,
    pub api_key: Option<String>,
    pub default_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenericHttpProvider {
    pub name: String,
    pub enabled: bool,
    pub base_url: String,
    pub api_key_env: String,
    pub api_key: Option<String>,
    pub default_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRouting {
    pub reasoning: TaskRoute,
    pub coding: TaskRoute,
    pub summarization: TaskRoute,
    pub vision: TaskRoute,
    pub classification: TaskRoute,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRoute {
    pub provider: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub max_actions_per_session: usize,
    pub max_artifacts_per_session: usize,
    pub max_cache_entries: usize,
    pub store_full_transcripts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchConfig {
    pub domain_allowlist: Vec<String>,
    pub domain_blocklist: Vec<String>,
    pub user_agent: String,
    pub per_host_delay_ms: u64,
    pub respect_robots: bool,
    pub max_content_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    pub default_timeout_secs: u64,
    pub max_stdout_kb: usize,
    pub max_stderr_kb: usize,
    pub sandbox_with_docker: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsConfig {
    pub enabled: bool,
    pub auto_load: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub config_path: PathBuf,
    pub state_dir: PathBuf,
    pub db_path: PathBuf,
    pub cache_dir: PathBuf,
    pub log_dir: PathBuf,
    pub writeups_dir: PathBuf,
    pub plugins_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub config: AppConfig,
    pub paths: RuntimePaths,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            safety: SafetyConfig::default(),
            providers: ProvidersConfig::default(),
            model_routing: ModelRouting::default(),
            memory: MemoryConfig::default(),
            research: ResearchConfig::default(),
            tools: ToolsConfig::default(),
            plugins: PluginsConfig::default(),
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            workspace_root: None,
            default_json_output: false,
            log_level: "info".to_string(),
        }
    }
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            allowed_paths: vec![PathBuf::from(".")],
            allowed_hosts: vec!["127.0.0.1".to_string(), "localhost".to_string()],
            allowed_ports: vec![80, 443, 8000, 8080, 1337],
            offline_only: false,
            research_web_enabled: true,
            require_confirmation_for_network: true,
            require_confirmation_for_exec: true,
            require_confirmation_for_install: true,
            max_runtime_per_action_secs: 120,
            max_parallel_actions: 2,
            redact_secrets_in_logs: true,
        }
    }
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        Self {
            fallback_local: true,
            request_timeout_secs: 45,
            retries: 2,
            backoff_base_ms: 500,
            max_input_tokens: 24_000,
            openai: OpenAiCompatProvider {
                enabled: false,
                base_url: "https://api.openai.com/v1".to_string(),
                api_key_env: "OPENAI_API_KEY".to_string(),
                api_key: None,
                default_model: "gpt-4.1-mini".to_string(),
            },
            openrouter: OpenAiCompatProvider {
                enabled: false,
                base_url: "https://openrouter.ai/api/v1".to_string(),
                api_key_env: "OPENROUTER_API_KEY".to_string(),
                api_key: None,
                default_model: "openai/gpt-4.1-mini".to_string(),
            },
            together: OpenAiCompatProvider {
                enabled: false,
                base_url: "https://api.together.xyz/v1".to_string(),
                api_key_env: "TOGETHER_API_KEY".to_string(),
                api_key: None,
                default_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo".to_string(),
            },
            moonshot: OpenAiCompatProvider {
                enabled: false,
                base_url: "https://api.moonshot.ai/v1".to_string(),
                api_key_env: "MOONSHOT_API_KEY".to_string(),
                api_key: None,
                default_model: "moonshot-v1-8k".to_string(),
            },
            anthropic: AnthropicProvider {
                name: "anthropic".to_string(),
                enabled: false,
                base_url: "https://api.anthropic.com/v1".to_string(),
                api_key_env: "ANTHROPIC_API_KEY".to_string(),
                api_key: None,
                default_model: "claude-3-5-sonnet-latest".to_string(),
            },
            gemini: GeminiProvider {
                enabled: false,
                api_key_env: "GEMINI_API_KEY".to_string(),
                api_key: None,
                base_url: "https://generativelanguage.googleapis.com/v1beta".to_string(),
                default_model: "gemini-1.5-flash".to_string(),
            },
            anthropic_compatible: Vec::new(),
            custom_openai_compatible: Vec::new(),
            generic_http: Vec::new(),
        }
    }
}

impl Default for ModelRouting {
    fn default() -> Self {
        Self {
            reasoning: TaskRoute {
                provider: "local".to_string(),
                model: None,
            },
            coding: TaskRoute {
                provider: "local".to_string(),
                model: None,
            },
            summarization: TaskRoute {
                provider: "local".to_string(),
                model: None,
            },
            vision: TaskRoute {
                provider: "local".to_string(),
                model: None,
            },
            classification: TaskRoute {
                provider: "local".to_string(),
                model: None,
            },
        }
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            max_actions_per_session: 2_500,
            max_artifacts_per_session: 10_000,
            max_cache_entries: 2_000,
            store_full_transcripts: false,
        }
    }
}

impl Default for ResearchConfig {
    fn default() -> Self {
        Self {
            domain_allowlist: Vec::new(),
            domain_blocklist: Vec::new(),
            user_agent: "0x0-ai/0.1 (+local-first ctf assistant)".to_string(),
            per_host_delay_ms: 500,
            respect_robots: true,
            max_content_bytes: 1_500_000,
        }
    }
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            default_timeout_secs: 120,
            max_stdout_kb: 2048,
            max_stderr_kb: 1024,
            sandbox_with_docker: false,
        }
    }
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_load: true,
        }
    }
}

impl RuntimePaths {
    pub fn from_project_dirs(project: &ProjectDirs) -> Self {
        let config_path = project.config_dir().join("config.toml");
        let state_dir = project.data_local_dir().to_path_buf();
        let cache_dir = project.cache_dir().to_path_buf();
        let log_dir = state_dir.join("logs");
        let writeups_dir = state_dir.join("writeups");
        let plugins_dir = state_dir.join("plugins");
        let db_path = state_dir.join("state.sqlite3");

        Self {
            config_path,
            state_dir,
            db_path,
            cache_dir,
            log_dir,
            writeups_dir,
            plugins_dir,
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            &self.state_dir,
            &self.cache_dir,
            &self.log_dir,
            &self.writeups_dir,
            &self.plugins_dir,
        ] {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        Ok(())
    }
}

pub fn load_runtime_config(explicit_config: Option<PathBuf>) -> Result<RuntimeConfig> {
    let project = ProjectDirs::from("ai", "0x0", "0x0-ai")
        .context("failed to resolve project directories")?;
    let mut paths = RuntimePaths::from_project_dirs(&project);

    if let Some(explicit) = explicit_config {
        paths.config_path = explicit;
    }

    paths.ensure_dirs()?;

    if !paths.config_path.exists() {
        let default = AppConfig::default();
        save_config(&paths.config_path, &default)?;
    }

    let config = load_config(&paths.config_path)?;
    Ok(RuntimeConfig { config, paths })
}

pub fn load_config(path: &Path) -> Result<AppConfig> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let cfg: AppConfig =
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    Ok(cfg)
}

pub fn save_config(path: &Path, cfg: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let serialized = toml::to_string_pretty(cfg).context("serializing config")?;
    fs::write(path, serialized).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
