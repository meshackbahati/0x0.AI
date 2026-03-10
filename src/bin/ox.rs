use clap::Parser;
use once_cell::sync::OnceCell;
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt};

use zerox0_ai::app::run;
use zerox0_ai::cli::Cli;
use zerox0_ai::config::load_runtime_config;

static LOG_GUARD: OnceCell<tracing_appender::non_blocking::WorkerGuard> = OnceCell::new();

fn main() {
    let cli = Cli::parse();
    init_logging(cli.config.clone()).ok();

    if let Err(err) = run(cli) {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn init_logging(config_path: Option<PathBuf>) -> anyhow::Result<()> {
    let runtime = load_runtime_config(config_path)?;
    let level = runtime.config.general.log_level;
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    let file_appender = tracing_appender::rolling::daily(&runtime.paths.log_dir, "0x0.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    let _ = LOG_GUARD.set(guard);

    fmt()
        .with_env_filter(env_filter)
        .with_writer(non_blocking)
        .with_target(true)
        .compact()
        .try_init()
        .ok();

    Ok(())
}
