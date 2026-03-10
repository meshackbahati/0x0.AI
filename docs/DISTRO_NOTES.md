# Distro Notes

## Ubuntu / Debian / Kali / Parrot

Common packages:

```bash
sudo apt-get install -y build-essential pkg-config libssl-dev
```

## Fedora

```bash
sudo dnf install -y gcc gcc-c++ make openssl-devel pkgconf-pkg-config
```

## Arch / Manjaro

```bash
sudo pacman -S --noconfirm base-devel openssl pkgconf
```

## Tooling Notes

`0x0 tools doctor` checks optional CTF tools.

`0x0 tools install <tool>` proposes and runs package-manager-specific commands with explicit approval.
