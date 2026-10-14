//! Local network addresses and the "is this peer on the LAN" check.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

use super::model::Address;

/// Interfaces a phone can never reach: VPN tunnels, VM/container bridges, AirDrop links.
fn is_virtual(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["utun", "bridge", "vmnet", "vboxnet", "docker", "veth", "awdl", "llw", "anpi", "gif", "stf", "tap", "tun", "zt", "tailscale"]
        .iter()
        .any(|p| n.starts_with(p))
        || n.contains("vethernet")
        || n.contains("virtualbox")
        || n.contains("vmware")
        || n.contains("hyper-v")
        || n.contains("wsl")
}

fn is_private_v4(ip: &Ipv4Addr) -> bool {
    ip.is_private() && !ip.is_loopback()
}

/// The interface that carries the default route. A UDP "connect" only
/// consults the routing table — no packet is sent. 203.0.113.1 is a
/// documentation-only address, so it always takes the default route.
pub fn primary_ip() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    sock.connect((Ipv4Addr::new(203, 0, 113, 1), 9)).ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if is_private_v4(&v4) => Some(v4),
        _ => None,
    }
}

/// Private IPv4 addresses a phone on the same Wi-Fi could use, best first.
pub fn lan_addresses() -> Vec<Address> {
    let mut found: Vec<(Ipv4Addr, String)> = Vec::new();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            if iface.is_loopback() || is_virtual(&iface.name) {
                continue;
            }
            if let if_addrs::IfAddr::V4(v4) = &iface.addr
                && is_private_v4(&v4.ip)
                && !found.iter().any(|(ip, _)| *ip == v4.ip)
            {
                found.push((v4.ip, iface.name.clone()));
            }
        }
    }
    let primary = primary_ip();
    found.sort_by_key(|(ip, name)| {
        let rank = if Some(*ip) == primary {
            0
        } else if name.starts_with("en") || name.contains("Wi-Fi") || name.contains("WLAN") || name.contains("Ethernet") || name.contains("以太网") {
            1
        } else {
            2
        };
        (rank, name.clone())
    });
    found.into_iter().map(|(ip, iface)| Address { ip: ip.to_string(), iface }).collect()
}

/// Only devices on the local network (or this machine) may talk to the LAN server.
pub fn is_lan_peer(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_peers() {
        for ok in ["192.168.1.20", "10.0.0.5", "172.16.3.4", "127.0.0.1", "169.254.1.1", "::1", "fe80::1", "fd00::1", "::ffff:192.168.1.2"] {
            assert!(is_lan_peer(ok.parse().unwrap()), "{ok}");
        }
        for bad in ["8.8.8.8", "172.32.0.1", "100.64.0.1", "2001:db8::1", "::ffff:1.1.1.1"] {
            assert!(!is_lan_peer(bad.parse().unwrap()), "{bad}");
        }
    }

    #[test]
    fn virtual_interfaces() {
        assert!(is_virtual("utun3"));
        assert!(is_virtual("bridge100"));
        assert!(is_virtual("vEthernet (WSL)"));
        assert!(!is_virtual("en0"));
        assert!(!is_virtual("Wi-Fi"));
    }
}
