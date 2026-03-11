pub mod ai;
pub mod blockchain;
pub mod cloud;
pub mod crypto;
pub mod forensics;
pub mod hardware;
pub mod misc;
pub mod mobile;
pub mod network;
pub mod osint;
pub mod pwn;
pub mod reverse;
pub mod stego;
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
    Mobile,
    Hardware,
    Blockchain,
    Cloud,
    Network,
    Ai,
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
            Self::Mobile => "mobile",
            Self::Hardware => "hardware",
            Self::Blockchain => "blockchain",
            Self::Cloud => "cloud",
            Self::Network => "network",
            Self::Ai => "ai",
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
    let mut score_mobile = 0_u32;
    let mut score_hardware = 0_u32;
    let mut score_blockchain = 0_u32;
    let mut score_cloud = 0_u32;
    let mut score_network = 0_u32;
    let mut score_ai = 0_u32;
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
        if p.ends_with(".pcap")
            || p.ends_with(".pcapng")
            || p.contains("network")
            || p.contains("packet")
            || summary.contains("tcp")
            || summary.contains("udp")
            || summary.contains("dns")
            || summary.contains("http request")
        {
            score_network += 2;
        }

        if p.ends_with(".png")
            || p.ends_with(".jpg")
            || p.ends_with(".jpeg")
            || p.ends_with(".bmp")
            || p.ends_with(".gif")
        {
            score_stego += 1;
        }
        if p.contains("stego")
            || summary.contains("stego")
            || summary.contains("lsb")
            || summary.contains("hidden data")
        {
            score_stego += 2;
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

        if p.contains("osint")
            || summary.contains("whois")
            || summary.contains("social")
            || summary.contains("geolocation")
            || summary.contains("dns")
            || summary.contains("username")
        {
            score_osint += 2;
        }

        if p.ends_with(".apk")
            || p.ends_with(".ipa")
            || p.ends_with(".dex")
            || p.ends_with(".smali")
            || p.contains("androidmanifest")
            || p.contains("mobile")
            || summary.contains("android")
            || summary.contains("ios")
            || summary.contains("frida")
        {
            score_mobile += 3;
        }

        if p.ends_with(".v")
            || p.ends_with(".sv")
            || p.ends_with(".vhdl")
            || p.ends_with(".bit")
            || p.contains("firmware")
            || p.contains("uart")
            || p.contains("jtag")
            || p.contains("hardware")
            || summary.contains("microcontroller")
            || summary.contains("firmware")
        {
            score_hardware += 3;
        }

        if p.ends_with(".sol")
            || p.ends_with(".vy")
            || p.contains("blockchain")
            || p.contains("smart-contract")
            || p.contains("ethereum")
            || summary.contains("smart contract")
            || summary.contains("evm")
            || summary.contains("web3")
        {
            score_blockchain += 3;
        }

        if p.ends_with(".tf")
            || p.ends_with(".tfvars")
            || p.contains("docker")
            || p.contains("kubernetes")
            || p.contains("k8s")
            || p.contains("helm")
            || p.contains("cloud")
            || summary.contains("iam")
            || summary.contains("s3")
            || summary.contains("azure")
            || summary.contains("gcp")
        {
            score_cloud += 2;
        }

        if p.ends_with(".onnx")
            || p.ends_with(".pt")
            || p.ends_with(".pth")
            || p.ends_with(".safetensors")
            || p.contains("tokenizer")
            || p.contains("prompt")
            || p.contains("model")
            || summary.contains("llm")
            || summary.contains("embedding")
            || summary.contains("inference")
        {
            score_ai += 3;
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
        (ChallengeCategory::Mobile, score_mobile),
        (ChallengeCategory::Hardware, score_hardware),
        (ChallengeCategory::Blockchain, score_blockchain),
        (ChallengeCategory::Cloud, score_cloud),
        (ChallengeCategory::Network, score_network),
        (ChallengeCategory::Ai, score_ai),
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
        ChallengeCategory::Forensics => forensics::plan(),
        ChallengeCategory::Stego => stego::plan(),
        ChallengeCategory::Osint => osint::plan(),
        ChallengeCategory::Mobile => mobile::plan(),
        ChallengeCategory::Hardware => hardware::plan(),
        ChallengeCategory::Blockchain => blockchain::plan(),
        ChallengeCategory::Cloud => cloud::plan(),
        ChallengeCategory::Network => network::plan(),
        ChallengeCategory::Ai => ai::plan(),
        ChallengeCategory::Misc | ChallengeCategory::Unknown => misc::plan(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_mobile_from_apk_artifacts() {
        let signals = vec![ArtifactSignal {
            path: "/tmp/challenge/app.apk".to_string(),
            kind: "binary".to_string(),
            size: 2048,
            summary: Some("Android package".to_string()),
        }];

        assert_eq!(infer_category(&signals), ChallengeCategory::Mobile);
    }

    #[test]
    fn infers_blockchain_from_solidity_artifacts() {
        let signals = vec![ArtifactSignal {
            path: "/tmp/challenge/Vault.sol".to_string(),
            kind: "source".to_string(),
            size: 1024,
            summary: Some("smart contract with EVM bytecode".to_string()),
        }];

        assert_eq!(infer_category(&signals), ChallengeCategory::Blockchain);
    }
}
