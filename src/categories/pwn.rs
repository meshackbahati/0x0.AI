use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Pwn,
        hypotheses: vec![
            HypothesisTemplate {
                text: "Binary likely vulnerable to stack/heap corruption with constrained primitives.".to_string(),
                confidence: 0.62,
            },
            HypothesisTemplate {
                text: "Mitigations (PIE/NX/Canary/RELRO) determine exploit strategy.".to_string(),
                confidence: 0.74,
            },
            HypothesisTemplate {
                text: "A pwntools skeleton can accelerate iterative local/lab exploit testing.".to_string(),
                confidence: 0.68,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description: "Triage ELF metadata and mitigations (file/readelf/checksec).".to_string(),
                command_preview: Some("file ./challenge && checksec --file=./challenge".to_string()),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Extract symbols/strings to discover win conditions and gadgets.".to_string(),
                command_preview: Some("strings -n 4 ./challenge | head -n 200".to_string()),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Generate pwntools-compatible exploit skeleton.".to_string(),
                command_preview: Some("python3 exploit.py --local".to_string()),
                requires_network: false,
                requires_install: false,
            },
        ],
    }
}
