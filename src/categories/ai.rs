use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Ai,
        hypotheses: vec![
            HypothesisTemplate {
                text:
                    "Prompt or retrieval boundary weaknesses may reveal protected instructions."
                        .to_string(),
                confidence: 0.6,
            },
            HypothesisTemplate {
                text:
                    "Model artifacts/configs may leak secrets, eval answers, or jailbreak bypasses."
                        .to_string(),
                confidence: 0.57,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description:
                    "Search prompts/config for hidden directives, guardrails, and secret tokens."
                        .to_string(),
                command_preview: Some(
                    "rg -n \"system|instruction|prompt|secret|policy|jailbreak|tool\" ."
                        .to_string(),
                ),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description:
                    "Inspect model/tokenizer artifacts for hardcoded hints and eval leaks."
                        .to_string(),
                command_preview: Some("python3 -c \"import json,glob;print(glob.glob('**/*token*', recursive=True)[:20])\"".to_string()),
                requires_network: false,
                requires_install: false,
            },
        ],
    }
}
