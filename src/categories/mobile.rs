use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Mobile,
        hypotheses: vec![
            HypothesisTemplate {
                text: "Sensitive constants or endpoints may be exposed in app resources."
                    .to_string(),
                confidence: 0.64,
            },
            HypothesisTemplate {
                text: "Static decompilation plus targeted dynamic hooks usually reveals flag path."
                    .to_string(),
                confidence: 0.58,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description: "Run APK/IPA triage and extract strings/resources.".to_string(),
                command_preview: Some(
                    "file app.apk && strings -n 4 app.apk | head -n 200".to_string(),
                ),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Decompile for deterministic review (jadx/apktool if available)."
                    .to_string(),
                command_preview: Some(
                    "jadx -d out app.apk || apktool d app.apk -o out".to_string(),
                ),
                requires_network: false,
                requires_install: false,
            },
        ],
    }
}
