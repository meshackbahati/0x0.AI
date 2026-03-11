use super::{ActionSuggestion, CategoryPlan, ChallengeCategory, HypothesisTemplate};

pub fn plan() -> CategoryPlan {
    CategoryPlan {
        category: ChallengeCategory::Hardware,
        hypotheses: vec![
            HypothesisTemplate {
                text:
                    "Firmware image likely contains plaintext secrets or recoverable config blobs."
                        .to_string(),
                confidence: 0.63,
            },
            HypothesisTemplate {
                text: "Signal/protocol clues (UART/SPI/JTAG) can expose debug or boot paths."
                    .to_string(),
                confidence: 0.47,
            },
        ],
        actions: vec![
            ActionSuggestion {
                description: "Inspect firmware structure and carve embedded files.".to_string(),
                command_preview: Some("file firmware.bin && binwalk -e firmware.bin".to_string()),
                requires_network: false,
                requires_install: false,
            },
            ActionSuggestion {
                description: "Run deterministic string/signature scans for keys and pins."
                    .to_string(),
                command_preview: Some(
                    "strings -n 4 firmware.bin | rg -n \"key|pin|flag|debug\"".to_string(),
                ),
                requires_network: false,
                requires_install: false,
            },
        ],
    }
}
