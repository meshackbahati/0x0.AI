use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Blockchain,
        hypotheses: vec![
            HypothesisTemplate {
                text: "Contract likely has a logic flaw in auth, arithmetic, or state transitions."
                    .to_string(),
                confidence: 0.67,
            },
            HypothesisTemplate {
                text:
                    "Unit-test style reproduction can confirm exploitability before chain interaction."
                        .to_string(),
                confidence: 0.53,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description:
                    "Statically review Solidity/Vyper for require checks and privileged branches."
                        .to_string(),
                command_preview: Some("rg -n \"require|revert|onlyOwner|delegatecall\" .".to_string()),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Compile/analyze contracts with local tooling when available."
                    .to_string(),
                command_preview: Some("solc --version && solc --ast-compact-json *.sol".to_string()),
                requires_network: false,
                requires_install: false,
            },
        ],
    }
}
