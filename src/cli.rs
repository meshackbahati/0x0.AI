use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Clone, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Parser)]
#[command(
    name = "0x0",
    version,
    about = "0x0.AI - local-first autonomous CTF assistant"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[arg(long, global = true)]
    pub json: bool,

    #[arg(long, global = true)]
    pub dry_run: bool,

    #[arg(long, global = true)]
    pub yes: bool,

    #[arg(long, global = true)]
    pub offline: bool,

    #[arg(long, global = true)]
    pub no_install: bool,

    #[arg(long, global = true, value_enum)]
    pub output: Option<OutputFormat>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Init(InitArgs),
    Setup(SetupArgs),
    Update(UpdateArgs),
    Scan(ScanArgs),
    Solve(SolveArgs),
    SolveAll(SolveAllArgs),
    Resume(ResumeArgs),
    Sessions(SessionsArgs),
    Research(ResearchArgs),
    Chat(ChatArgs),
    Note(NoteArgs),
    Tools(ToolsArgs),
    Providers(ProvidersArgs),
    Web(WebArgs),
    Writeup(WriteupArgs),
    Replay(ReplayArgs),
    Config(ConfigArgs),
    Stats(StatsArgs),
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(default_value = ".")]
    pub path: PathBuf,

    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct SetupArgs {
    #[arg(long)]
    pub provider: Option<String>,

    #[arg(long)]
    pub api_key: Option<String>,

    #[arg(long)]
    pub api_key_env: Option<String>,

    #[arg(long)]
    pub model: Option<String>,

    #[arg(long)]
    pub base_url: Option<String>,

    #[arg(long)]
    pub route: Option<RouteTask>,

    #[arg(long)]
    pub compat: Option<ProviderCompat>,

    #[arg(long, default_value_t = false)]
    pub non_interactive: bool,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    #[arg(long, default_value_t = false)]
    pub system: bool,

    #[arg(long, default_value_t = false)]
    pub user: bool,

    #[arg(long)]
    pub branch: Option<String>,

    #[arg(long)]
    pub reference: Option<String>,

    #[arg(long, default_value_t = false)]
    pub prefer_commit: bool,

    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    pub path: PathBuf,

    #[arg(long)]
    pub session_id: Option<String>,

    #[arg(long, default_value_t = true)]
    pub recursive: bool,

    #[arg(long, default_value_t = 10 * 1024 * 1024)]
    pub max_read_bytes: usize,
}

#[derive(Debug, Args)]
pub struct SolveArgs {
    pub path: PathBuf,

    #[arg(long)]
    pub session_id: Option<String>,

    #[arg(long, default_value_t = 8)]
    pub max_steps: usize,

    #[arg(long)]
    pub web: bool,

    #[arg(long)]
    pub approve_network: bool,

    #[arg(long)]
    pub approve_exec: bool,

    #[arg(long)]
    pub approve_install: bool,
}

#[derive(Debug, Args)]
pub struct SolveAllArgs {
    pub path: PathBuf,

    #[arg(long, default_value_t = 6)]
    pub max_steps: usize,

    #[arg(long)]
    pub web: bool,

    #[arg(long)]
    pub approve_network: bool,

    #[arg(long)]
    pub approve_exec: bool,

    #[arg(long)]
    pub approve_install: bool,

    #[arg(long, default_value_t = 40)]
    pub max_challenges: usize,
}

#[derive(Debug, Args)]
pub struct ResumeArgs {
    pub session_id: String,

    #[arg(long, default_value_t = false)]
    pub continue_solve: bool,

    #[arg(long, default_value_t = 4)]
    pub max_steps: usize,

    #[arg(long)]
    pub web: bool,

    #[arg(long)]
    pub approve_network: bool,

    #[arg(long)]
    pub approve_exec: bool,
}

#[derive(Debug, Args)]
pub struct SessionsArgs {
    #[arg(long, default_value_t = 20)]
    pub limit: usize,

    #[arg(long)]
    pub status: Option<String>,

    #[arg(long)]
    pub category: Option<String>,
}

#[derive(Debug, Args)]
pub struct ResearchArgs {
    pub query: String,

    #[arg(long, default_value_t = true)]
    pub local: bool,

    #[arg(long, default_value_t = false)]
    pub web: bool,

    #[arg(long)]
    pub session_id: Option<String>,

    #[arg(long, default_value_t = 5)]
    pub max_results: usize,

    #[arg(long)]
    pub approve_network: bool,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum RouteTask {
    Reasoning,
    Coding,
    Summarization,
    Vision,
    Classification,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum ProviderCompat {
    Openai,
    Anthropic,
    Generic,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ChatApprovalMode {
    All,
    Risky,
}

#[derive(Debug, Args)]
pub struct ChatArgs {
    #[arg(long)]
    pub session_id: Option<String>,

    #[arg(long)]
    pub provider: Option<String>,

    #[arg(long)]
    pub system: Option<String>,

    #[arg(long)]
    pub prompt: Option<String>,

    #[arg(long, action = ArgAction::Set, default_value_t = true)]
    pub autonomous: bool,

    #[arg(long, value_enum, default_value_t = ChatApprovalMode::Risky)]
    pub approval_mode: ChatApprovalMode,

    #[arg(long, default_value_t = 8)]
    pub max_agent_steps: usize,

    #[arg(long, default_value_t = false)]
    pub web: bool,

    #[arg(long)]
    pub approve_network: bool,

    #[arg(long)]
    pub approve_exec: bool,

    #[arg(long, default_value_t = true)]
    pub show_actions: bool,

    #[arg(long, default_value_t = 200)]
    pub max_turns: usize,
}

impl Default for ChatArgs {
    fn default() -> Self {
        Self {
            session_id: None,
            provider: None,
            system: None,
            prompt: None,
            autonomous: true,
            approval_mode: ChatApprovalMode::Risky,
            max_agent_steps: 8,
            web: false,
            approve_network: false,
            approve_exec: false,
            show_actions: true,
            max_turns: 200,
        }
    }
}

#[derive(Debug, Args)]
pub struct NoteArgs {
    pub session_id: String,

    #[arg(required = true)]
    pub text: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ToolsArgs {
    #[command(subcommand)]
    pub command: ToolsCommand,
}

#[derive(Debug, Subcommand)]
pub enum ToolsCommand {
    Doctor(ToolsDoctorArgs),
    Install(ToolsInstallArgs),
}

#[derive(Debug, Args)]
pub struct ToolsDoctorArgs {
    #[arg(long, default_value_t = false)]
    pub verbose: bool,
}

#[derive(Debug, Args)]
pub struct ToolsInstallArgs {
    pub tool: String,

    #[arg(long)]
    pub approve_install: bool,
}

#[derive(Debug, Args)]
pub struct ProvidersArgs {
    #[command(subcommand)]
    pub command: ProvidersCommand,
}

#[derive(Debug, Subcommand)]
pub enum ProvidersCommand {
    Test(ProvidersTestArgs),
    Configure(ProvidersConfigureArgs),
    Models(ProvidersModelsArgs),
    Use(ProvidersUseArgs),
}

#[derive(Debug, Args)]
pub struct ProvidersTestArgs {
    #[arg(long)]
    pub provider: Option<String>,

    #[arg(long, default_value = "Return a one-line status response.")]
    pub prompt: String,
}

#[derive(Debug, Args)]
pub struct ProvidersConfigureArgs {
    pub provider: String,

    #[arg(long)]
    pub api_key: Option<String>,

    #[arg(long)]
    pub api_key_env: Option<String>,

    #[arg(long)]
    pub model: Option<String>,

    #[arg(long)]
    pub base_url: Option<String>,

    #[arg(long, default_value_t = false)]
    pub enable: bool,

    #[arg(long, default_value_t = false)]
    pub disable: bool,

    #[arg(long)]
    pub route: Option<RouteTask>,

    #[arg(long)]
    pub compat: Option<ProviderCompat>,
}

#[derive(Debug, Args)]
pub struct ProvidersModelsArgs {
    #[arg(long)]
    pub provider: Option<String>,
}

#[derive(Debug, Args)]
pub struct ProvidersUseArgs {
    #[arg(long)]
    pub task: RouteTask,

    #[arg(long)]
    pub provider: String,

    #[arg(long)]
    pub model: String,
}

#[derive(Debug, Args)]
pub struct WebArgs {
    #[command(subcommand)]
    pub command: WebCommand,
}

#[derive(Debug, Subcommand)]
pub enum WebCommand {
    Map(WebMapArgs),
    Replay(WebReplayArgs),
    Template(WebTemplateArgs),
}

#[derive(Debug, Args)]
pub struct WebMapArgs {
    pub target: String,

    #[arg(long)]
    pub session_id: Option<String>,

    #[arg(long)]
    pub approve_network: bool,

    #[arg(long)]
    pub approve_exec: bool,

    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct WebReplayArgs {
    pub target: String,

    #[arg(long, default_value = "GET")]
    pub method: String,

    #[arg(long, default_value = "/")]
    pub path: String,

    #[arg(long)]
    pub header: Vec<String>,

    #[arg(long)]
    pub data: Option<String>,

    #[arg(long)]
    pub session_id: Option<String>,

    #[arg(long)]
    pub approve_network: bool,

    #[arg(long)]
    pub approve_exec: bool,
}

#[derive(Debug, Args)]
pub struct WebTemplateArgs {
    pub target: String,

    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct WriteupArgs {
    pub session_id: String,

    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ReplayArgs {
    pub session_id: String,

    #[arg(long, default_value_t = 100)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Edit,
    Show,
}

#[derive(Debug, Args)]
pub struct StatsArgs {
    #[arg(long)]
    pub session_id: Option<String>,
}
