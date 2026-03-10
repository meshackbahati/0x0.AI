use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PackageManager {
    Apt,
    Dnf,
    Pacman,
    Yay,
    Paru,
    Zypper,
    Pipx,
    Pip,
    Cargo,
    Go,
    Npm,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallPlan {
    pub manager: PackageManager,
    pub tool: String,
    pub package: String,
    pub command: Vec<String>,
    pub note: String,
}

pub fn detect_package_manager() -> PackageManager {
    if which::which("apt-get").is_ok() {
        return PackageManager::Apt;
    }
    if which::which("dnf").is_ok() {
        return PackageManager::Dnf;
    }
    if which::which("pacman").is_ok() {
        return PackageManager::Pacman;
    }
    if which::which("zypper").is_ok() {
        return PackageManager::Zypper;
    }
    if which::which("yay").is_ok() {
        return PackageManager::Yay;
    }
    if which::which("paru").is_ok() {
        return PackageManager::Paru;
    }
    PackageManager::Unknown
}

pub fn build_install_plan(tool: &str, manager_hint: Option<PackageManager>) -> InstallPlan {
    let manager = manager_hint.unwrap_or_else(detect_package_manager);
    let package = package_name_for(tool, &manager);

    let command = match manager {
        PackageManager::Apt => vec![
            "sudo".to_string(),
            "apt-get".to_string(),
            "install".to_string(),
            "-y".to_string(),
            package.clone(),
        ],
        PackageManager::Dnf => vec![
            "sudo".to_string(),
            "dnf".to_string(),
            "install".to_string(),
            "-y".to_string(),
            package.clone(),
        ],
        PackageManager::Pacman => vec![
            "sudo".to_string(),
            "pacman".to_string(),
            "-S".to_string(),
            "--noconfirm".to_string(),
            package.clone(),
        ],
        PackageManager::Zypper => vec![
            "sudo".to_string(),
            "zypper".to_string(),
            "install".to_string(),
            "-y".to_string(),
            package.clone(),
        ],
        PackageManager::Yay => vec![
            "yay".to_string(),
            "-S".to_string(),
            "--noconfirm".to_string(),
            package.clone(),
        ],
        PackageManager::Paru => vec![
            "paru".to_string(),
            "-S".to_string(),
            "--noconfirm".to_string(),
            package.clone(),
        ],
        PackageManager::Pipx => vec!["pipx".to_string(), "install".to_string(), package.clone()],
        PackageManager::Pip => vec!["python3".to_string(), "-m".to_string(), "pip".to_string(), "install".to_string(), package.clone()],
        PackageManager::Cargo => vec!["cargo".to_string(), "install".to_string(), package.clone()],
        PackageManager::Go => vec!["go".to_string(), "install".to_string(), package.clone()],
        PackageManager::Npm => vec!["npm".to_string(), "install".to_string(), "-g".to_string(), package.clone()],
        PackageManager::Unknown => vec![tool.to_string()],
    };

    let note = match manager {
        PackageManager::Unknown => {
            "No package manager auto-detected. Provide install command manually.".to_string()
        }
        _ => "Always review this command before execution in your lab environment.".to_string(),
    };

    InstallPlan {
        manager,
        tool: tool.to_string(),
        package,
        command,
        note,
    }
}

fn package_name_for(tool: &str, manager: &PackageManager) -> String {
    let t = tool.to_ascii_lowercase();

    let mut common = HashMap::new();
    common.insert("rg", "ripgrep");
    common.insert("ripgrep", "ripgrep");
    common.insert("jq", "jq");
    common.insert("yq", "yq");
    common.insert("exiftool", "exiftool");
    common.insert("binwalk", "binwalk");
    common.insert("foremost", "foremost");
    common.insert("zsteg", "zsteg");
    common.insert("steghide", "steghide");
    common.insert("gdb", "gdb");
    common.insert("objdump", "binutils");
    common.insert("readelf", "binutils");
    common.insert("radare2", "radare2");
    common.insert("tshark", "tshark");
    common.insert("tcpdump", "tcpdump");
    common.insert("capinfos", "wireshark-common");
    common.insert("hashcat", "hashcat");
    common.insert("john", "john");
    common.insert("nmap", "nmap");
    common.insert("ffuf", "ffuf");
    common.insert("curl", "curl");
    common.insert("httpie", "httpie");
    common.insert("pdftotext", "poppler-utils");
    common.insert("tesseract", "tesseract-ocr");

    let fallback = common
        .get(t.as_str())
        .copied()
        .unwrap_or_else(|| tool.trim());

    match manager {
        PackageManager::Pacman | PackageManager::Yay | PackageManager::Paru => match fallback {
            "poppler-utils" => "poppler",
            "tesseract-ocr" => "tesseract",
            "wireshark-common" => "wireshark-cli",
            other => other,
        }
        .to_string(),
        PackageManager::Dnf => match fallback {
            "poppler-utils" => "poppler-utils",
            "wireshark-common" => "wireshark-cli",
            other => other,
        }
        .to_string(),
        _ => fallback.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_plan_contains_tool() {
        let plan = build_install_plan("rg", Some(PackageManager::Apt));
        assert!(plan.command.iter().any(|x| x == "ripgrep"));
    }
}
