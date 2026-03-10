use anyhow::{Context, Result};
use reqwest::blocking::{Client, Response};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

use crate::config::ResearchConfig;
use crate::research::{Citation, ResearchHit};
use crate::storage::StateStore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebFetch {
    pub url: String,
    pub title: Option<String>,
    pub content: String,
    pub snippet: String,
    pub from_cache: bool,
}

#[derive(Debug, Clone)]
struct RobotsPolicy {
    disallow: Vec<String>,
}

pub struct WebResearcher {
    cfg: ResearchConfig,
    client: Client,
    last_access: Mutex<HashMap<String, Instant>>,
    robots: Mutex<HashMap<String, RobotsPolicy>>,
}

impl WebResearcher {
    pub fn new(cfg: ResearchConfig) -> Result<Self> {
        let client = Client::builder()
            .user_agent(cfg.user_agent.clone())
            .timeout(Duration::from_secs(25))
            .build()
            .context("building web client")?;
        Ok(Self {
            cfg,
            client,
            last_access: Mutex::new(HashMap::new()),
            robots: Mutex::new(HashMap::new()),
        })
    }

    pub fn search(
        &self,
        query: &str,
        max_results: usize,
        store: &StateStore,
    ) -> Result<Vec<ResearchHit>> {
        if looks_like_url(query) {
            let Some(page) = self.fetch_url(query, store)? else {
                return Ok(Vec::new());
            };
            return Ok(vec![self.to_hit(page)]);
        }

        let search_url = format!(
            "https://duckduckgo.com/html/?q={}",
            url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>()
        );

        let html = self.fetch_raw_html(&search_url)?;
        let urls = extract_search_urls(&html, max_results * 3);

        let mut hits = Vec::new();
        let mut seen = HashSet::new();

        for url in urls {
            if hits.len() >= max_results {
                break;
            }
            if !seen.insert(url.clone()) {
                continue;
            }
            if let Some(page) = self.fetch_url(&url, store)? {
                hits.push(self.to_hit(page));
            }
        }

        Ok(hits)
    }

    pub fn fetch_url(&self, url: &str, store: &StateStore) -> Result<Option<WebFetch>> {
        let parsed = Url::parse(url).context("parsing url")?;
        let Some(host) = parsed.host_str() else {
            return Ok(None);
        };

        if !self.domain_allowed(host) {
            return Ok(None);
        }

        if !self.robot_allowed(&parsed)? {
            return Ok(None);
        }

        if let Some(cached) = store.get_web_cache(url)? {
            return Ok(Some(WebFetch {
                url: cached.url,
                title: cached.title,
                snippet: truncate(&cached.content, 240),
                content: cached.content,
                from_cache: true,
            }));
        }

        self.apply_rate_limit(host);

        let response = self
            .client
            .get(url)
            .send()
            .with_context(|| format!("GET {url}"))?;

        if !response.status().is_success() {
            return Ok(None);
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();

        let raw = read_response_limited(response, self.cfg.max_content_bytes)?;
        let html = String::from_utf8_lossy(&raw).to_string();

        let (title, content) = if content_type.contains("html") {
            extract_readable_text(&html)
        } else {
            (None, html)
        };

        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let hash = hex::encode(hasher.finalize());

        store.put_web_cache(url, title.as_deref(), &content, &hash)?;

        Ok(Some(WebFetch {
            url: url.to_string(),
            title,
            snippet: truncate(&content, 240),
            content,
            from_cache: false,
        }))
    }

    fn to_hit(&self, page: WebFetch) -> ResearchHit {
        ResearchHit {
            title: page.title.clone(),
            snippet: page.snippet.clone(),
            citation: Citation {
                source_type: "web".to_string(),
                source: page.url,
                locator: None,
                snippet: page.snippet,
            },
        }
    }

    fn fetch_raw_html(&self, url: &str) -> Result<String> {
        let parsed = Url::parse(url).context("parsing raw html url")?;
        if let Some(host) = parsed.host_str() {
            self.apply_rate_limit(host);
        }
        let mut resp = self.client.get(url).send().with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            return Ok(String::new());
        }

        let mut text = String::new();
        resp.read_to_string(&mut text).ok();
        Ok(text)
    }

    fn domain_allowed(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        if self
            .cfg
            .domain_blocklist
            .iter()
            .any(|b| host.ends_with(&b.to_ascii_lowercase()))
        {
            return false;
        }

        if self.cfg.domain_allowlist.is_empty() {
            return true;
        }

        self.cfg
            .domain_allowlist
            .iter()
            .any(|a| host.ends_with(&a.to_ascii_lowercase()))
    }

    fn apply_rate_limit(&self, host: &str) {
        let delay = Duration::from_millis(self.cfg.per_host_delay_ms);
        if delay.is_zero() {
            return;
        }

        let mut guard = self.last_access.lock().expect("lock");
        if let Some(last) = guard.get(host) {
            let elapsed = last.elapsed();
            if elapsed < delay {
                thread::sleep(delay - elapsed);
            }
        }
        guard.insert(host.to_string(), Instant::now());
    }

    fn robot_allowed(&self, url: &Url) -> Result<bool> {
        if !self.cfg.respect_robots {
            return Ok(true);
        }

        let Some(host) = url.host_str() else {
            return Ok(false);
        };

        let cache_key = format!("{}://{}", url.scheme(), host);
        let path = url.path();

        if let Some(policy) = self.robots.lock().expect("lock").get(&cache_key).cloned() {
            return Ok(robots_allows(path, &policy));
        }

        let robots_url = format!("{}/robots.txt", cache_key);
        let text = self
            .client
            .get(&robots_url)
            .send()
            .ok()
            .and_then(|r| r.text().ok())
            .unwrap_or_default();

        let policy = parse_robots(&text);
        let allowed = robots_allows(path, &policy);
        self.robots.lock().expect("lock").insert(cache_key, policy);
        Ok(allowed)
    }
}

fn extract_search_urls(html: &str, max: usize) -> Vec<String> {
    let doc = Html::parse_document(html);
    let selector_primary = Selector::parse("a.result__a").expect("selector");
    let selector_fallback = Selector::parse("a[href]").expect("selector");

    let mut urls = Vec::new();

    for node in doc.select(&selector_primary) {
        if let Some(href) = node.value().attr("href")
            && let Some(url) = normalize_search_href(href)
        {
            urls.push(url);
            if urls.len() >= max {
                return urls;
            }
        }
    }

    for node in doc.select(&selector_fallback) {
        if let Some(href) = node.value().attr("href")
            && let Some(url) = normalize_search_href(href)
        {
            urls.push(url);
            if urls.len() >= max {
                break;
            }
        }
    }

    urls
}

fn normalize_search_href(href: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }

    if href.starts_with("/l/?")
        && let Ok(base) = Url::parse("https://duckduckgo.com")
        && let Ok(url) = base.join(href)
        && let Some((_, target)) = url.query_pairs().find(|(k, _)| k == "uddg")
    {
        return Some(target.to_string());
    }

    None
}

fn parse_robots(content: &str) -> RobotsPolicy {
    let mut in_global = false;
    let mut disallow = Vec::new();

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let lower = line.to_ascii_lowercase();
        if lower.starts_with("user-agent:") {
            let value = line.split_once(':').map(|(_, v)| v.trim()).unwrap_or("");
            in_global = value == "*";
            continue;
        }

        if in_global && lower.starts_with("disallow:") {
            let value = line.split_once(':').map(|(_, v)| v.trim()).unwrap_or("");
            if !value.is_empty() {
                disallow.push(value.to_string());
            }
        }
    }

    RobotsPolicy { disallow }
}

fn robots_allows(path: &str, policy: &RobotsPolicy) -> bool {
    for disallow in &policy.disallow {
        if disallow == "/" {
            return false;
        }
        if !disallow.is_empty() && path.starts_with(disallow) {
            return false;
        }
    }
    true
}

fn extract_readable_text(html: &str) -> (Option<String>, String) {
    let doc = Html::parse_document(html);
    let title = Selector::parse("title")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|el| el.text().collect::<Vec<_>>().join(" ").trim().to_string())
        .filter(|s| !s.is_empty());

    let selectors = ["main", "article", "section", "p", "li", "pre", "code", "h1", "h2", "h3"];

    let mut lines = Vec::new();
    for sel in selectors {
        if let Ok(selector) = Selector::parse(sel) {
            for node in doc.select(&selector) {
                let line = node.text().collect::<Vec<_>>().join(" ");
                let line = line.trim();
                if !line.is_empty() {
                    lines.push(line.to_string());
                }
            }
        }
    }

    if lines.is_empty()
        && let Ok(body_sel) = Selector::parse("body")
        && let Some(body) = doc.select(&body_sel).next()
    {
        let text = body.text().collect::<Vec<_>>().join(" ");
        lines.push(text);
    }

    let text = lines
        .into_iter()
        .map(|s| s.replace('\n', " "))
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    (title, truncate(&text, 20_000))
}

fn read_response_limited(mut response: Response, max: usize) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(max.min(64 * 1024));
    let mut chunk = [0_u8; 8192];
    loop {
        let n = response.read(&mut chunk).context("reading response body")?;
        if n == 0 {
            break;
        }
        let remaining = max.saturating_sub(buf.len());
        if remaining == 0 {
            break;
        }
        let take = remaining.min(n);
        buf.extend_from_slice(&chunk[..take]);
        if take < n {
            break;
        }
    }
    Ok(buf)
}

fn looks_like_url(query: &str) -> bool {
    query.starts_with("http://") || query.starts_with("https://")
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
    fn parses_robots() {
        let robots = "User-agent: *\nDisallow: /private\n";
        let parsed = parse_robots(robots);
        assert!(!robots_allows("/private/data", &parsed));
        assert!(robots_allows("/public", &parsed));
    }
}
