use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use url::Url;

use crate::policy::{Approvals, PolicyEngine};
use crate::tools::{ToolManager, ToolRunRequest};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebTarget {
    pub base_url: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebProbe {
    pub path: String,
    pub status_code: Option<u16>,
    pub title: Option<String>,
    pub content_length: usize,
    pub command_preview: String,
    pub status: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzTemplate {
    pub name: String,
    pub description: String,
    pub command_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebMapReport {
    pub target: WebTarget,
    pub probes: Vec<WebProbe>,
    pub discovered_params: Vec<String>,
    pub fuzz_templates: Vec<FuzzTemplate>,
    pub payload_notebook_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebReplayReport {
    pub target: WebTarget,
    pub method: String,
    pub path: String,
    pub status_code: Option<u16>,
    pub command_preview: String,
    pub stdout_excerpt: String,
    pub stderr_excerpt: String,
}

pub fn parse_target(input: &str) -> Result<WebTarget> {
    let url = Url::parse(input).context("target must be full URL, e.g. http://127.0.0.1:8080")?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        bail!("unsupported URL scheme: {scheme}")
    }

    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("target URL missing host"))?
        .to_string();

    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("could not determine target port"))?;

    let base_url = format!("{}://{}:{}", scheme, host, port);
    Ok(WebTarget {
        base_url,
        host,
        port,
    })
}

pub fn map_target(
    target: &WebTarget,
    tools: &ToolManager,
    policy: &PolicyEngine,
    approvals: Approvals,
    notebook_out: &Path,
) -> Result<WebMapReport> {
    policy.ensure_network_allowed(approvals, &target.host, Some(target.port), false)?;
    policy.ensure_exec_allowed(approvals, "curl")?;

    let probe_paths = [
        "/",
        "/robots.txt",
        "/sitemap.xml",
        "/login",
        "/register",
        "/admin",
        "/api",
        "/api/health",
        "/graphql",
        "/.well-known/security.txt",
    ];

    let mut probes = Vec::new();
    let mut params = BTreeSet::new();

    for p in probe_paths {
        let url = format!("{}{}", target.base_url, p);
        let args = vec!["-sS".to_string(), "-i".to_string(), url];
        let run = tools.run(ToolRunRequest {
            program: "curl".to_string(),
            args,
            cwd: None,
            timeout_secs: Some(20),
        })?;

        let status_code = parse_status_code(&run.stdout);
        let title = extract_title(&run.stdout);
        let body = extract_body(&run.stdout);
        for param in extract_params(&body) {
            params.insert(param);
        }

        probes.push(WebProbe {
            path: p.to_string(),
            status_code,
            title,
            content_length: body.len(),
            command_preview: run.command_preview,
            status: run.status,
            excerpt: truncate(&body, 300),
        });
    }

    let fuzz_templates = build_fuzz_templates(target, &params);
    let notebook_path = write_payload_notebook(target, notebook_out, &params)?;

    Ok(WebMapReport {
        target: target.clone(),
        probes,
        discovered_params: params.into_iter().collect(),
        fuzz_templates,
        payload_notebook_path: notebook_path,
    })
}

pub fn replay_request(
    target: &WebTarget,
    tools: &ToolManager,
    policy: &PolicyEngine,
    approvals: Approvals,
    method: &str,
    path: &str,
    headers: &[String],
    data: Option<&str>,
) -> Result<WebReplayReport> {
    policy.ensure_network_allowed(approvals, &target.host, Some(target.port), false)?;
    policy.ensure_exec_allowed(approvals, "curl")?;

    let full = format!(
        "{}{}{}",
        target.base_url,
        if path.starts_with('/') { "" } else { "/" },
        path
    );

    let mut args = vec![
        "-sS".to_string(),
        "-i".to_string(),
        "-X".to_string(),
        method.to_uppercase(),
    ];

    for h in headers {
        args.push("-H".to_string());
        args.push(h.clone());
    }

    if let Some(payload) = data {
        args.push("--data".to_string());
        args.push(payload.to_string());
    }

    args.push(full);

    let run = tools.run(ToolRunRequest {
        program: "curl".to_string(),
        args,
        cwd: None,
        timeout_secs: Some(20),
    })?;

    Ok(WebReplayReport {
        target: target.clone(),
        method: method.to_uppercase(),
        path: path.to_string(),
        status_code: parse_status_code(&run.stdout),
        command_preview: run.command_preview,
        stdout_excerpt: truncate(&run.stdout, 600),
        stderr_excerpt: truncate(&run.stderr, 400),
    })
}

pub fn generate_templates_and_notebook(
    target: &WebTarget,
    out_dir: &Path,
) -> Result<(Vec<FuzzTemplate>, String)> {
    let params = BTreeSet::new();
    let templates = build_fuzz_templates(target, &params);
    let notebook = write_payload_notebook(target, out_dir, &params)?;
    Ok((templates, notebook))
}

fn build_fuzz_templates(target: &WebTarget, params: &BTreeSet<String>) -> Vec<FuzzTemplate> {
    let mut templates = vec![
        FuzzTemplate {
            name: "endpoint-discovery".to_string(),
            description: "Limited endpoint discovery template for approved lab host.".to_string(),
            command_preview: format!(
                "ffuf -u {}/FUZZ -w /usr/share/seclists/Discovery/Web-Content/common.txt -mc all -fs 0",
                target.base_url
            ),
        },
        FuzzTemplate {
            name: "param-discovery".to_string(),
            description: "Parameter name discovery template against one endpoint.".to_string(),
            command_preview: format!(
                "ffuf -u {}/search?FUZZ=test -w /usr/share/seclists/Discovery/Web-Content/burp-parameter-names.txt -mc all",
                target.base_url
            ),
        },
        FuzzTemplate {
            name: "request-replay".to_string(),
            description: "Replay a crafted request quickly with curl.".to_string(),
            command_preview: format!(
                "curl -i -X POST {}/login -H 'Content-Type: application/x-www-form-urlencoded' --data 'username=test&password=test'",
                target.base_url
            ),
        },
    ];

    if !params.is_empty() {
        let joined = params.iter().take(8).cloned().collect::<Vec<_>>().join(",");
        templates.push(FuzzTemplate {
            name: "known-params".to_string(),
            description: "Parameter notebook from discovered names.".to_string(),
            command_preview: format!("# discovered params: {joined}"),
        });
    }

    templates
}

fn write_payload_notebook(
    target: &WebTarget,
    out_dir: &Path,
    params: &BTreeSet<String>,
) -> Result<String> {
    fs::create_dir_all(out_dir)?;
    let path = out_dir.join(format!(
        "payload-notebook-{}.md",
        target.host.replace('.', "_")
    ));

    let mut content = String::new();
    content.push_str("# Web Payload Notebook (Authorized Lab Use Only)\n\n");
    content.push_str(&format!("Target: `{}`\n\n", target.base_url));
    content.push_str(
        "Use this only for explicit CTF/lab targets you own or are authorized to test.\n\n",
    );

    content.push_str("## Discovered Parameters\n");
    if params.is_empty() {
        content.push_str("- none yet\n\n");
    } else {
        for p in params {
            content.push_str(&format!("- `{}`\n", p));
        }
        content.push('\n');
    }

    content.push_str("## SQL Injection Checks\n");
    content.push_str("- `' OR '1'='1`\n");
    content.push_str("- `1 UNION SELECT NULL`\n\n");

    content.push_str("## XSS Checks\n");
    content.push_str("- `<script>alert(1)</script>`\n");
    content.push_str("- `\"><img src=x onerror=alert(1)>`\n\n");

    content.push_str("## SSTI Checks\n");
    content.push_str("- `{{7*7}}`\n");
    content.push_str("- `${7*7}`\n\n");

    content.push_str("## Path Traversal Checks\n");
    content.push_str("- `../../../../etc/passwd`\n");
    content.push_str("- `..%2f..%2f..%2f..%2fetc%2fpasswd`\n\n");

    content.push_str("## Command Injection Checks\n");
    content.push_str("- `;id`\n");
    content.push_str("- `|id`\n\n");

    content.push_str("## Notes\n- Record status codes and response deltas per payload.\n");

    fs::write(&path, content)?;
    Ok(path.display().to_string())
}

fn parse_status_code(response: &str) -> Option<u16> {
    for line in response.lines() {
        if line.starts_with("HTTP/") {
            let mut parts = line.split_whitespace();
            let _ = parts.next();
            if let Some(code) = parts.next()
                && let Ok(v) = code.parse::<u16>()
            {
                return Some(v);
            }
        }
    }
    None
}

fn extract_title(response: &str) -> Option<String> {
    let body = extract_body(response);
    let re = Regex::new(r"(?is)<title>(.*?)</title>").expect("title regex");
    re.captures(&body)
        .and_then(|cap| cap.get(1).map(|m| m.as_str().trim().to_string()))
        .filter(|s| !s.is_empty())
}

fn extract_body(response: &str) -> String {
    if let Some((_, body)) = response.split_once("\r\n\r\n") {
        return body.to_string();
    }
    if let Some((_, body)) = response.split_once("\n\n") {
        return body.to_string();
    }
    response.to_string()
}

fn extract_params(body: &str) -> Vec<String> {
    let mut out = BTreeSet::new();

    let query_re = Regex::new(r"[?&]([a-zA-Z0-9_]{1,40})=").expect("query regex");
    for cap in query_re.captures_iter(body) {
        if let Some(m) = cap.get(1) {
            out.insert(m.as_str().to_string());
        }
    }

    let name_re = Regex::new(r#"name=[\"']([a-zA-Z0-9_\-]{1,40})[\"']"#).expect("name regex");
    for cap in name_re.captures_iter(body) {
        if let Some(m) = cap.get(1) {
            out.insert(m.as_str().to_string());
        }
    }

    out.into_iter().collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_target_url() {
        let t = parse_target("http://127.0.0.1:8080").expect("target");
        assert_eq!(t.host, "127.0.0.1");
        assert_eq!(t.port, 8080);
    }

    #[test]
    fn extracts_http_status() {
        let s = "HTTP/1.1 302 Found\r\nLocation: /login\r\n\r\n";
        assert_eq!(parse_status_code(s), Some(302));
    }
}
