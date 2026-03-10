use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Crypto,
        hypotheses: vec![
            HypothesisTemplate {
                text: "Cipher may be weak RSA variant (small e, related moduli, or partial key exposure).".to_string(),
                confidence: 0.58,
            },
            HypothesisTemplate {
                text: "Challenge likely combines layered encodings before core cryptanalysis.".to_string(),
                confidence: 0.41,
            },
            HypothesisTemplate {
                text: "Known attack playbook (CRT reuse, common modulus, Wiener's, Fermat) may apply.".to_string(),
                confidence: 0.54,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description: "Extract numeric parameters and check RSA sanity constraints.".to_string(),
                command_preview: Some("rg -n \"(n|e|c|p|q)\" challenge/*".to_string()),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Generate modular arithmetic helper script scaffold.".to_string(),
                command_preview: Some("0x0 note <session> 'Generate RSA helper scaffold'".to_string()),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Try deterministic attacks before heavy brute-force or lattice tooling.".to_string(),
                command_preview: None,
                requires_network: false,
                requires_install: false,
            },
        ],
    }
}
