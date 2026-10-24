use sysinfo::System;

#[derive(Debug, Clone, Default)]
pub struct VpnInfo {
    pub detected: bool,
    pub matches: Vec<String>,
}

impl VpnInfo {
    pub fn display_name(&self) -> String {
        if self.matches.is_empty() {
            "VPN".to_string()
        } else {
            self.matches.join(", ")
        }
    }
}

const VPN_PROCESS_PATTERNS: &[&str] = &[
    // Commercial VPN clients
    "nordvpn",
    "expressvpn",
    "protonvpn",
    "protonvpn-service",
    "mullvad",
    "surfshark",
    "cyberghost",
    "ipvanish",
    "windscribe",
    "privateinternetaccess",
    "pia",
    "tunnelbear",
    "hide.me",
    "hideme",
    "azirevpn",
    "ivpn",
    "airvpn",
    "purevpn",
    "vyprvpn",
    "hotspotshield",
    "speedify",
    "hidemyass",
    "astrill",
    "perfectprivacy",
    "privatevpn",
    "strongvpn",
    "ipredator",
    // Open protocols
    "openvpn",
    "openconnect",
    "wireguard",
    "wg-quick",
    // Cloaking / proxy clients
    "shadowsocks",
    "v2ray",
    "xray",
    "clash",
    "sing-box",
    "trojan",
    "hysteria",
    // Mesh / WARP (soft signals - often dev tools)
    "tailscale",
    "tailscaled",
    "warp-svc",
    "warp-cli",
    "cloudflare warp",
];

pub fn detect() -> VpnInfo {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let mut matches: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (_pid, process) in sys.processes() {
        let raw_name = process.name().to_string_lossy().to_string();
        let lower = raw_name.to_lowercase();
        for pattern in VPN_PROCESS_PATTERNS {
            if lower.contains(pattern) {
                let key = pattern.to_string();
                if seen.insert(key) {
                    matches.push(raw_name.clone());
                }
                break;
            }
        }
    }

    matches.sort();
    matches.dedup();

    VpnInfo {
        detected: !matches.is_empty(),
        matches,
    }
}