use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Network,
        hypotheses: vec![
            HypothesisTemplate {
                text: "Traffic likely contains protocol misuse or leaked credentials in cleartext."
                    .to_string(),
                confidence: 0.65,
            },
            HypothesisTemplate {
                text:
                    "Reassembling conversations and filtering key streams should expose flag flow."
                        .to_string(),
                confidence: 0.59,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description: "Summarize packet captures and key protocols.".to_string(),
                command_preview: Some(
                    "capinfos capture.pcap && tshark -r capture.pcap -q".to_string(),
                ),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Extract suspicious payload strings and credentials.".to_string(),
                command_preview: Some(
                    "tshark -r capture.pcap -Y \"http || dns || tcp\" -T fields -e data"
                        .to_string(),
                ),
                requires_network: false,
                requires_install: false,
            },
        ],
    }
}
