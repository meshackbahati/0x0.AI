use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Web,
        hypotheses: vec![
            HypothesisTemplate {
                text: "Input validation or auth logic likely exposes challenge path.".to_string(),
                confidence: 0.56,
            },
            HypothesisTemplate {
                text: "Parameter tampering and replay against approved lab targets may uncover flag endpoint.".to_string(),
                confidence: 0.52,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description: "Map endpoints passively with approved host/port constraints.".to_string(),
                command_preview: Some("curl -i http://<approved-host>:<port>/".to_string()),
                requires_network: true,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Create request replay notebook and parameter fuzz templates.".to_string(),
                command_preview: None,
                requires_network: true,
                requires_install: false,
            },
        ],
    }
}
