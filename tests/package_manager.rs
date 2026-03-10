use zerox0_ai::tools::package::{PackageManager, build_install_plan};

#[test]
fn apt_maps_rg_to_ripgrep() {
    let plan = build_install_plan("rg", Some(PackageManager::Apt));
    let joined = plan.command.join(" ");
    assert!(joined.contains("ripgrep"));
}

#[test]
fn pacman_maps_poppler_utils_to_poppler() {
    let plan = build_install_plan("pdftotext", Some(PackageManager::Pacman));
    let joined = plan.command.join(" ");
    assert!(joined.contains("poppler"));
}
