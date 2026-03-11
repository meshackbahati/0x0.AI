use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Stego,
        hypotheses: vec![
            HypothesisTemplate {
                text: "Flag data may be hidden in image channels, metadata, or embedded payloads."
                    .to_string(),
                confidence: 0.66,
            },
            HypothesisTemplate {
                text:
                    "Multiple extraction tools may be required because encoding method is unknown."
                        .to_string(),
                confidence: 0.51,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description: "Inspect metadata and signatures (file/exiftool/strings).".to_string(),
                command_preview: Some(
                    "file ./artifact && exiftool ./artifact && strings -n 4 ./artifact".to_string(),
                ),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Attempt deterministic stego extraction with local tools.".to_string(),
                command_preview: Some(
                    "zsteg ./artifact || steghide extract -sf ./artifact".to_string(),
                ),
                requires_network: false,
                requires_install: false,
            },
        ],
    }
}
