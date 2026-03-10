use anyhow::Result;

use super::{Provider, ProviderRequest, ProviderResponse, TaskType};
use crate::util::{chunk_text, estimate_tokens};

pub struct LocalProvider;

impl LocalProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Provider for LocalProvider {
    fn name(&self) -> &str {
        "local"
    }

    fn default_model(&self) -> &str {
        "heuristic-local-v1"
    }

    fn generate(
        &self,
        req: &ProviderRequest,
        mut stream: Option<&mut dyn FnMut(&str)>,
    ) -> Result<ProviderResponse> {
        let text = match req.task_type {
            TaskType::Classification => classify_prompt(&req.prompt),
            TaskType::Summarization => summarize_prompt(&req.prompt),
            TaskType::Coding => format!(
                "Local coding fallback: produce a minimal deterministic scaffold first.\n\nFocus:\n- Validate inputs\n- Log key decisions\n- Keep actions reproducible"
            ),
            TaskType::Vision => {
                "Local vision fallback: OCR/vision model not configured. Use tesseract/exiftool where available.".to_string()
            }
            TaskType::Reasoning => reasoning_hint(&req.prompt),
        };

        if let Some(sink) = stream.as_mut() {
            for c in chunk_text(&text, 42) {
                sink(&c);
            }
        }

        Ok(ProviderResponse {
            provider: "local".to_string(),
            model: self.default_model().to_string(),
            prompt_tokens_est: estimate_tokens(&req.prompt),
            completion_tokens_est: estimate_tokens(&text),
            text,
        })
    }
}

fn summarize_prompt(prompt: &str) -> String {
    let lines: Vec<&str> = prompt
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(12)
        .collect();

    if lines.is_empty() {
        return "No content provided for summarization.".to_string();
    }

    let mut out = String::from("Summary (local fallback):\n");
    for (idx, line) in lines.iter().take(5).enumerate() {
        out.push_str(&format!("{}. {}\n", idx + 1, truncate(line, 120)));
    }
    out.push_str("Confidence: low-to-medium (heuristic mode).\n");
    out
}

fn classify_prompt(prompt: &str) -> String {
    let p = prompt.to_ascii_lowercase();
    if p.contains("rsa") || p.contains("cipher") || p.contains("modulus") {
        return "Likely category: crypto (heuristic).".to_string();
    }
    if p.contains("elf") || p.contains("overflow") || p.contains("canary") {
        return "Likely category: pwn (heuristic).".to_string();
    }
    if p.contains("pcap") || p.contains("disk") || p.contains("metadata") {
        return "Likely category: forensics (heuristic).".to_string();
    }
    if p.contains("http") || p.contains("endpoint") || p.contains("cookie") {
        return "Likely category: web (heuristic).".to_string();
    }
    "Likely category: misc/rev (heuristic, uncertain).".to_string()
}

fn reasoning_hint(prompt: &str) -> String {
    format!(
        "Local reasoning fallback: prioritize deterministic checks, then minimal experiments, then heavier analysis.\nInput preview: {}",
        truncate(prompt.trim(), 180)
    )
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
