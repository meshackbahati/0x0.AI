use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Osint,
        hypotheses: vec![
            HypothesisTemplate {
                text: "Challenge likely depends on correlating leaked identifiers across sources."
                    .to_string(),
                confidence: 0.61,
            },
            HypothesisTemplate {
                text: "Small metadata clues can pivot into a direct target account/domain."
                    .to_string(),
                confidence: 0.55,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description: "Extract entities (emails, domains, handles) from provided artifacts."
                    .to_string(),
                command_preview: Some(
                    "rg -n \"@|https?://|discord|telegram|twitter\" .".to_string(),
                ),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description:
                    "Run only approved passive lookups (whois/dig/curl) and log citations."
                        .to_string(),
                command_preview: Some("whois <domain> && dig <domain>".to_string()),
                requires_network: true,
                requires_install: false,
            },
        ],
    }
}
