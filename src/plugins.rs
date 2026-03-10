use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::tools::{ToolManager, ToolRunRequest, ToolRunResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub manifest_path: String,
    pub manifest: PluginManifest,
}

pub struct PluginManager {
    dir: PathBuf,
}

impl PluginManager {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn discover(&self) -> Result<Vec<PluginInfo>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut plugins = Vec::new();

        for entry in fs::read_dir(&self.dir).with_context(|| format!("reading {}", self.dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if !path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
            {
                continue;
            }

            let raw = fs::read_to_string(&path)?;
            let manifest: PluginManifest = toml::from_str(&raw)
                .with_context(|| format!("parsing plugin manifest {}", path.display()))?;

            plugins.push(PluginInfo {
                manifest_path: path.display().to_string(),
                manifest,
            });
        }

        plugins.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
        Ok(plugins)
    }

    pub fn run_plugin(
        &self,
        name: &str,
        extra_args: &[String],
        tool_manager: &ToolManager,
        cwd: Option<&Path>,
    ) -> Result<ToolRunResult> {
        let plugins = self.discover()?;
        let plugin = plugins
            .into_iter()
            .find(|p| p.manifest.name == name)
            .ok_or_else(|| anyhow::anyhow!("plugin not found: {name}"))?;

        let mut args = plugin.manifest.args;
        args.extend(extra_args.to_vec());

        let req = ToolRunRequest {
            program: plugin.manifest.command,
            args,
            cwd: cwd.map(Path::to_path_buf),
            timeout_secs: None,
        };

        tool_manager.run(req)
    }
}
