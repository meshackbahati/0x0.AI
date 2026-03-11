use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Cloud,
        hypotheses: vec![
            HypothesisTemplate {
                text: "Misconfigured IAM/storage/network policy likely creates an escalation path."
                    .to_string(),
                confidence: 0.62,
            },
            HypothesisTemplate {
                text: "IaC templates usually reveal trust boundaries and exploitable assumptions."
                    .to_string(),
                confidence: 0.56,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description:
                    "Audit Terraform/Kubernetes/Docker manifests for public exposure and secrets."
                        .to_string(),
                command_preview: Some(
                    "rg -n \"iam|policy|public|bucket|secret|token|kube|docker\" .".to_string(),
                ),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Validate cloud assumptions with controlled local reproduction."
                    .to_string(),
                command_preview: Some("docker compose config || kubectl kustomize .".to_string()),
                requires_network: false,
                requires_install: false,
            },
        ],
    }
}
