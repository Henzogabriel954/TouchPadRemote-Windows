use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};

use if_addrs::IfAddr;

pub struct NetworkInfo;

impl NetworkInfo {
    pub fn get_local_ip_addresses() -> Vec<String> {
        get_local_ip_addresses()
    }

    pub fn resolve_best_local_ip_for(remote_ip: IpAddr) -> Option<String> {
        resolve_best_local_ip_for(remote_ip)
    }
}

struct Candidate {
    ip: Ipv4Addr,
    mask: u32,
    score: i32,
}

pub fn get_local_ip_addresses() -> Vec<String> {
    let mut seen = HashSet::new();
    candidates()
        .into_iter()
        .filter(|entry| seen.insert(entry.ip))
        .map(|entry| entry.ip.to_string())
        .collect()
}

pub fn resolve_best_local_ip_for(remote_ip: IpAddr) -> Option<String> {
    if remote_ip.is_loopback() {
        return Some(Ipv4Addr::LOCALHOST.to_string());
    }
    let IpAddr::V4(remote) = remote_ip else {
        return None;
    };
    let candidates = candidates();
    let matches =
        |entry: &Candidate, mask: u32| u32::from(entry.ip) & mask == u32::from(remote) & mask;
    candidates
        .iter()
        .find(|entry| entry.mask != 0 && matches(entry, entry.mask))
        .or_else(|| candidates.iter().find(|entry| matches(entry, 0xffff_0000)))
        .map(|entry| entry.ip.to_string())
}

fn candidates() -> Vec<Candidate> {
    let mut result: Vec<_> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|iface| {
            let IfAddr::V4(address) = iface.addr else {
                return None;
            };
            let ip = address.ip;
            if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_multicast() {
                return None;
            }
            Some(Candidate {
                ip,
                mask: address.netmask.into(),
                score: score(&iface.name, ip),
            })
        })
        .collect();
    result.sort_by_key(|entry| {
        (
            entry.score,
            std::cmp::Reverse(entry.mask.count_ones()),
            entry.ip,
        )
    });
    result
}

fn score(name: &str, ip: Ipv4Addr) -> i32 {
    let name = name.to_ascii_lowercase();
    let matches = |keywords: &[&str]| keywords.iter().any(|keyword| name.contains(keyword));
    let adapter = if matches(&[
        "virtual",
        "vmware",
        "vbox",
        "hyper-v",
        "vethernet",
        "wsl",
        "docker",
        "tun",
        "tap",
        "teredo",
        "bluetooth",
        "npcap",
        "hamachi",
        "tailscale",
        "zerotier",
        "wireguard",
        "wintun",
        "vpn",
        "nordlynx",
        "mullvad",
        "proton",
        "warp",
    ]) {
        200
    } else {
        0
    };
    let medium = if matches(&["wi-fi", "wifi", "wlan", "wireless", "802.11"]) {
        -30
    } else if matches(&["ethernet", "eth", "realtek", "intel"]) {
        -15
    } else {
        0
    };
    let subnet = match ip.octets() {
        [192, 168, ..] => 0,
        [10, ..] => 10,
        [172, 16..=31, ..] => 20,
        _ => 40,
    };
    adapter + medium + subnet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_lan_adapters_rank_ahead_of_virtual_adapters() {
        assert!(
            score("Wi-Fi", Ipv4Addr::new(192, 168, 1, 2))
                < score("Ethernet", Ipv4Addr::new(10, 0, 0, 2))
        );
        for name in [
            "WSL",
            "Docker",
            "Tailscale",
            "WireGuard",
            "Cloudflare WARP",
            "VPN",
        ] {
            assert!(
                score("Ethernet", Ipv4Addr::new(172, 16, 1, 2))
                    < score(name, Ipv4Addr::new(192, 168, 1, 2))
            );
        }
    }
}
