use crate::config::SafetyConfig;
use crate::util::normalize_host;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default)]
pub struct Approvals {
    pub network: bool,
    pub exec: bool,
    pub install: bool,
}

#[derive(Debug, Clone)]
pub struct PolicyEngine {
    cfg: SafetyConfig,
    cwd: PathBuf,
}

impl PolicyEngine {
    pub fn new(cfg: SafetyConfig) -> Result<Self> {
        let cwd = std::env::current_dir().context("resolving current directory")?;
        Ok(Self { cfg, cwd })
    }

    pub fn config(&self) -> &SafetyConfig {
        &self.cfg
    }

    pub fn is_path_allowed(&self, path: &Path) -> bool {
        let target = canonicalize_lossy(path);
        self.cfg
            .allowed_paths
            .iter()
            .map(|p| {
                if p.is_absolute() {
                    canonicalize_lossy(p)
                } else {
                    canonicalize_lossy(&self.cwd.join(p))
                }
            })
            .any(|allowed| target.starts_with(&allowed))
    }

    pub fn ensure_path_allowed(&self, path: &Path) -> Result<()> {
        if self.is_path_allowed(path) {
            Ok(())
        } else {
            bail!(
                "path {} is outside allowlist; update safety.allowed_paths in config",
                path.display()
            )
        }
    }

    pub fn ensure_exec_allowed(&self, approvals: Approvals, program: &str) -> Result<()> {
        if self.cfg.require_confirmation_for_exec && !approvals.exec {
            bail!("execution blocked for {program}; pass explicit approval (--approve-exec/--yes)")
        }
        Ok(())
    }

    pub fn ensure_install_allowed(&self, approvals: Approvals, tool: &str) -> Result<()> {
        if self.cfg.require_confirmation_for_install && !approvals.install {
            bail!("install blocked for {tool}; pass explicit approval (--approve-install/--yes)")
        }
        Ok(())
    }

    pub fn ensure_network_allowed(
        &self,
        approvals: Approvals,
        host: &str,
        port: Option<u16>,
        passive_research: bool,
    ) -> Result<()> {
        if self.cfg.offline_only {
            bail!("network action blocked: offline_only is enabled")
        }

        if passive_research && !self.cfg.research_web_enabled {
            bail!("web research is disabled by policy")
        }

        if self.cfg.require_confirmation_for_network && !approvals.network {
            bail!(
                "network action blocked for host {host}; pass explicit approval (--approve-network/--yes)"
            )
        }

        if !passive_research {
            let normalized = normalize_host(host).unwrap_or_else(|| host.to_ascii_lowercase());
            if !self
                .cfg
                .allowed_hosts
                .iter()
                .any(|h| h.eq_ignore_ascii_case(&normalized))
            {
                bail!("host {host} is not in safety.allowed_hosts")
            }

            if let Some(p) = port
                && !self.cfg.allowed_ports.contains(&p)
            {
                bail!("port {p} is not in safety.allowed_ports")
            }
        }

        Ok(())
    }
}

fn canonicalize_lossy(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_allowlist_works() {
        let cfg = SafetyConfig {
            allowed_paths: vec![PathBuf::from(".")],
            ..SafetyConfig::default()
        };
        let policy = PolicyEngine::new(cfg).expect("policy");
        assert!(policy.is_path_allowed(Path::new("./src")));
    }
}
