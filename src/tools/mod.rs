pub mod package;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;
use wait_timeout::ChildExt;

use crate::config::ToolsConfig;
use crate::util::shell_preview;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatus {
    pub name: String,
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolRunRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRunResult {
    pub command_preview: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u128,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub dry_run: bool,
}

pub struct ToolManager {
    cfg: ToolsConfig,
    dry_run: bool,
}

impl ToolManager {
    pub fn new(cfg: ToolsConfig, dry_run: bool) -> Self {
        Self { cfg, dry_run }
    }

    pub fn discover_default_tools(&self) -> Vec<ToolStatus> {
        let defaults = [
            "strings", "file", "grep", "rg", "jq", "yq", "binwalk", "foremost", "exiftool",
            "zsteg", "steghide", "gdb", "checksec", "objdump", "readelf", "radare2", "python3",
            "sage", "gp", "tshark", "tcpdump", "capinfos", "hashcat", "john", "nmap", "ffuf",
            "curl", "http", "docker", "podman", "pdftotext", "tesseract",
        ];
        self.discover_tools(&defaults)
    }

    pub fn discover_tools(&self, names: &[&str]) -> Vec<ToolStatus> {
        names.iter().map(|n| discover_tool(n)).collect()
    }

    pub fn run(&self, req: ToolRunRequest) -> Result<ToolRunResult> {
        let preview = shell_preview(&req.program, &req.args);
        if self.dry_run {
            return Ok(ToolRunResult {
                command_preview: preview,
                status: "dry-run".to_string(),
                exit_code: None,
                duration_ms: 0,
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                dry_run: true,
            });
        }

        let timeout_secs = req.timeout_secs.unwrap_or(self.cfg.default_timeout_secs);
        let start = Instant::now();

        let mut command = Command::new(&req.program);
        command.args(&req.args);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        if let Some(cwd) = &req.cwd {
            command.current_dir(cwd);
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("spawning command: {preview}"))?;

        let status_opt = child.wait_timeout(std::time::Duration::from_secs(timeout_secs))?;
        let timed_out = status_opt.is_none();

        if timed_out {
            let _ = child.kill();
        }

        let output = child
            .wait_with_output()
            .with_context(|| format!("capturing command output: {preview}"))?;

        let duration_ms = start.elapsed().as_millis();
        let stdout = truncate_bytes(output.stdout, self.cfg.max_stdout_kb * 1024);
        let stderr = truncate_bytes(output.stderr, self.cfg.max_stderr_kb * 1024);
        let status = if timed_out {
            "timeout"
        } else if output.status.success() {
            "ok"
        } else {
            "error"
        }
        .to_string();

        Ok(ToolRunResult {
            command_preview: preview,
            status,
            exit_code: output.status.code(),
            duration_ms,
            stdout,
            stderr,
            timed_out,
            dry_run: false,
        })
    }
}

fn truncate_bytes(bytes: Vec<u8>, max: usize) -> String {
    if bytes.len() <= max {
        String::from_utf8_lossy(&bytes).to_string()
    } else {
        let mut out = String::from_utf8_lossy(&bytes[..max]).to_string();
        out.push_str("\n[...truncated...]");
        out
    }
}

fn discover_tool(name: &str) -> ToolStatus {
    let path = which::which(name).ok();
    let version = path
        .as_ref()
        .and_then(|_| try_version(name).ok())
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.lines().next().unwrap_or_default().to_string());

    ToolStatus {
        name: name.to_string(),
        available: path.is_some(),
        path: path.map(|p| p.display().to_string()),
        version,
    }
}

fn try_version(name: &str) -> Result<String> {
    let output = Command::new(name)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match output {
        Ok(out) => {
            let text = if out.stdout.is_empty() {
                String::from_utf8_lossy(&out.stderr).to_string()
            } else {
                String::from_utf8_lossy(&out.stdout).to_string()
            };
            Ok(text)
        }
        Err(_) => {
            let out = Command::new(name)
                .arg("-V")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()?;
            let text = if out.stdout.is_empty() {
                String::from_utf8_lossy(&out.stderr).to_string()
            } else {
                String::from_utf8_lossy(&out.stdout).to_string()
            };
            Ok(text)
        }
    }
}
