use anyhow::{Context, Result};
use atty::Stream;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::time::Duration;
use url::Url;

pub fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

pub fn now_utc_rfc3339() -> String {
    now_utc().to_rfc3339()
}

pub fn hash_file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 16 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn read_text_prefix(path: &Path, max_bytes: usize) -> Result<String> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::with_capacity(max_bytes.min(1024 * 1024));
    let mut chunk = [0_u8; 4096];
    while buf.len() < max_bytes {
        let to_read = (max_bytes - buf.len()).min(chunk.len());
        let n = reader.read(&mut chunk[..to_read])?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

pub fn shell_preview(program: &str, args: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&shell_escape::escape(program.into()).to_string());
    for arg in args {
        out.push(' ');
        out.push_str(&shell_escape::escape(arg.as_str().into()).to_string());
    }
    out
}

pub fn confirm(question: &str, auto_yes: bool) -> Result<bool> {
    if auto_yes {
        return Ok(true);
    }
    if !atty::is(Stream::Stdin) {
        return Ok(false);
    }

    print!("{} [y/N]: ", question);
    io::stdout().flush()?;

    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let normalized = line.trim().to_ascii_lowercase();
    Ok(matches!(normalized.as_str(), "y" | "yes"))
}

pub fn normalize_host(input: &str) -> Option<String> {
    if let Ok(url) = Url::parse(input) {
        return url.host_str().map(|h| h.to_ascii_lowercase());
    }
    let input = input.trim().to_ascii_lowercase();
    if input.is_empty() { None } else { Some(input) }
}

pub fn estimate_tokens(s: &str) -> usize {
    (s.len() / 4).max(1)
}

pub fn chunk_text(input: &str, max_chars: usize) -> Vec<String> {
    if input.is_empty() {
        return Vec::new();
    }
    if input.len() <= max_chars {
        return vec![input.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start = 0;
    while start < input.len() {
        let end = (start + max_chars).min(input.len());
        let mut boundary = end;
        while !input.is_char_boundary(boundary) {
            boundary -= 1;
        }
        chunks.push(input[start..boundary].to_string());
        start = boundary;
    }
    chunks
}

pub fn clamp_duration_secs(secs: u64, default: u64, max: u64) -> Duration {
    let s = if secs == 0 { default } else { secs.min(max) };
    Duration::from_secs(s)
}
