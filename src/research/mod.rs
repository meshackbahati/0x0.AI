pub mod local;
pub mod web;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub source_type: String,
    pub source: String,
    pub locator: Option<String>,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchHit {
    pub title: Option<String>,
    pub snippet: String,
    pub citation: Citation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchResult {
    pub query: String,
    pub local_hits: Vec<ResearchHit>,
    pub web_hits: Vec<ResearchHit>,
    pub inferences: Vec<String>,
}
