use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Text,
    Json,
}

impl OutputMode {
    pub fn from_flags(json: bool, explicit_json: Option<bool>) -> Self {
        if explicit_json.unwrap_or(json) {
            Self::Json
        } else {
            Self::Text
        }
    }
}

pub fn print_structured<T: Serialize>(
    mode: OutputMode,
    value: &T,
    text_fallback: &str,
) -> Result<()> {
    match mode {
        OutputMode::Text => {
            println!("{text_fallback}");
        }
        OutputMode::Json => {
            println!("{}", serde_json::to_string_pretty(value)?);
        }
    }
    Ok(())
}

pub fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
