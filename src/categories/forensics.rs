use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Forensics,
        hypotheses: vec![
            HypothesisTemplate {
                text: "Evidence likely split across metadata and carved artifacts.".to_string(),
                confidence: 0.64,
            },
            HypothesisTemplate {
                text: "Timeline reconstruction may reveal the intended extraction path."
                    .to_string(),
                confidence: 0.49,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description: "Perform quick metadata triage (file/exiftool/capinfos/binwalk)."
                    .to_string(),
                command_preview: Some("file ./artifact && exiftool ./artifact".to_string()),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Carve and inspect embedded files in isolated workspace.".to_string(),
                command_preview: Some("binwalk -e ./artifact".to_string()),
                requires_network: false,
                requires_install: false,
            },
        ],
    }
}
