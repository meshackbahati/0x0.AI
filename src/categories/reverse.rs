use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Reverse,
        hypotheses: vec![
            HypothesisTemplate {
                text: "Target likely hides flag transformation logic in function graph branches."
                    .to_string(),
                confidence: 0.57,
            },
            HypothesisTemplate {
                text:
                    "String and import analysis can quickly expose algorithm families or API clues."
                        .to_string(),
                confidence: 0.63,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description: "Run static triage (file, strings, objdump/readelf).".to_string(),
                command_preview: Some("file ./target && readelf -h ./target".to_string()),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Record decompiler notes and annotate suspect functions.".to_string(),
                command_preview: None,
                requires_network: false,
                requires_install: false,
            },
        ],
    }
}
