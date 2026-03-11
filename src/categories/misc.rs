use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Misc,
        hypotheses: vec![
            HypothesisTemplate {
                text: "Challenge may require layered encoding/decoding and format conversion."
                    .to_string(),
                confidence: 0.43,
            },
            HypothesisTemplate {
                text: "Custom parser script can eliminate repetitive manual trial-and-error."
                    .to_string(),
                confidence: 0.39,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description: "Probe for common encodings (base64/hex/rot/compression).".to_string(),
                command_preview: Some("python3 - <<'PY'\nprint('decode helpers')\nPY".to_string()),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Build one-off parser/automation scratchpad and log dead ends."
                    .to_string(),
                command_preview: None,
                requires_network: false,
                requires_install: false,
            },
        ],
    }
}
