pub mod crypto;
pub mod forensics;
pub mod misc;
pub mod pwn;
pub mod reverse;
pub mod web;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChallengeCategory {
    Crypto,
    Pwn,
    Reverse,
    Web,
    Misc,
    Forensics,
    Stego,
    Osint,
    Unknown,
}

impl ChallengeCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Crypto => "crypto",
            Self::Pwn => "pwn",
            Self::Reverse => "rev",
            Self::Web => "web",
            Self::Misc => "misc",
            Self::Forensics => "forensics",
            Self::Stego => "stego",
            Self::Osint => "osint",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionSuggestion {
    pub description: String,
    pub command_preview: Option<String>,
    pub requires_network: bool,
    pub requires_install: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypothesisTemplate {
    pub text: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryPlan {
    pub category: ChallengeCategory,
    pub hypotheses: Vec<HypothesisTemplate>,
    pub actions: Vec<ActionSuggestion>,
}

#[derive(Debug, Clone)]
pub struct ArtifactSignal {
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub summary: Option<String>,
}

pub fn infer_category(signals: &[ArtifactSignal]) -> ChallengeCategory {
    if signals.is_empty() {
        return ChallengeCategory::Unknown;
    }

    let mut score_crypto = 0_u32;
    let mut score_pwn = 0_u32;
    let mut score_rev = 0_u32;
    let mut score_web = 0_u32;
    let mut score_forensics = 0_u32;
    let mut score_stego = 0_u32;
    let mut score_osint = 0_u32;
    let mut score_misc = 0_u32;

    for signal in signals {
        let p = signal.path.to_ascii_lowercase();
        let kind = signal.kind.to_ascii_lowercase();
        let summary = signal
            .summary
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();

        if p.ends_with(".pcap")
            || p.ends_with(".pcapng")
            || p.ends_with(".dd")
            || p.ends_with(".img")
            || p.ends_with(".raw")
            || p.ends_with(".evtx")
            || p.ends_with(".log")
            || kind.contains("archive")
        {
            score_forensics += 2;
        }

        if p.ends_with(".png")
            || p.ends_with(".jpg")
            || p.ends_with(".jpeg")
            || p.ends_with(".bmp")
            || p.ends_with(".gif")
        {
            score_stego += 1;
        }

        if p.ends_with(".rsa")
            || p.contains("cipher")
            || p.contains("crypto")
            || summary.contains("modulus")
            || summary.contains("ciphertext")
            || summary.contains("prime")
        {
            score_crypto += 2;
        }

        if p.ends_with(".elf")
            || p.contains("libc")
            || p.contains("rop")
            || p.contains("pwn")
            || summary.contains("elf")
            || summary.contains("canary")
        {
            score_pwn += 2;
        }

        if p.ends_with(".exe")
            || p.ends_with(".dll")
            || p.ends_with(".so")
            || p.ends_with(".bin")
            || p.contains("reverse")
            || summary.contains("symbol")
            || summary.contains("function")
        {
            score_rev += 2;
        }

        if p.ends_with(".js")
            || p.ends_with(".php")
            || p.ends_with(".html")
            || p.ends_with(".sql")
            || p.contains("web")
            || summary.contains("http")
            || summary.contains("endpoint")
        {
            score_web += 2;
        }

        if p.contains("osint") || summary.contains("whois") || summary.contains("social") {
            score_osint += 2;
        }

        if p.ends_with(".txt") || p.ends_with(".md") {
            score_misc += 1;
        }
    }

    let mut best = (ChallengeCategory::Misc, score_misc);
    for candidate in [
        (ChallengeCategory::Crypto, score_crypto),
        (ChallengeCategory::Pwn, score_pwn),
        (ChallengeCategory::Reverse, score_rev),
        (ChallengeCategory::Web, score_web),
        (ChallengeCategory::Forensics, score_forensics),
        (ChallengeCategory::Stego, score_stego),
        (ChallengeCategory::Osint, score_osint),
        (ChallengeCategory::Misc, score_misc),
    ] {
        if candidate.1 > best.1 {
            best = candidate;
        }
    }

    if best.1 == 0 {
        ChallengeCategory::Unknown
    } else {
        best.0
    }
}

pub fn plan_for(category: ChallengeCategory) -> CategoryPlan {
    match category {
        ChallengeCategory::Crypto => crypto::plan(),
        ChallengeCategory::Pwn => pwn::plan(),
        ChallengeCategory::Reverse => reverse::plan(),
        ChallengeCategory::Web => web::plan(),
        ChallengeCategory::Forensics | ChallengeCategory::Stego => forensics::plan(),
        ChallengeCategory::Osint | ChallengeCategory::Misc | ChallengeCategory::Unknown => misc::plan(),
    }
}
