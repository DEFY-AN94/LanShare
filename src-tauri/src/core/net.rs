//! Local network addresses, this computer's names and the "is this peer on the LAN" check.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

use super::model::Address;
use super::tls;

/// Interfaces a phone can never reach: VPN tunnels, VM/container bridges, AirDrop links.
fn is_virtual(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["utun", "bridge", "vmnet", "vboxnet", "docker", "awdl", "llw", "anpi", "gif", "stf", "tap", "tun", "zt", "tailscale"]
        .iter()
        .any(|p| n.starts_with(p))
        || (n.starts_with("veth") && !n.starts_with("vethernet"))
        // Hyper-V NAT/internal switches only; an External switch carries the real LAN IP.
        || (n.contains("vethernet") && (n.contains("default switch") || n.contains("wsl") || n.contains("nat")))
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
    let primary = primary_ip();
    let mut found: Vec<(Ipv4Addr, String)> = Vec::new();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            let if_addrs::IfAddr::V4(v4) = &iface.addr else { continue };
            // A Hyper-V External switch carries the real LAN address, so keep a
            // vEthernet adapter when it holds the default route (VPN tunnels stay out).
            let hyperv_primary = Some(v4.ip) == primary && iface.name.to_ascii_lowercase().contains("vethernet");
            if iface.is_loopback() || (is_virtual(&iface.name) && !hyperv_primary) {
                continue;
            }
            if is_private_v4(&v4.ip) && !found.iter().any(|(ip, _)| *ip == v4.ip) {
                found.push((v4.ip, iface.name.clone()));
            }
        }
    }
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

/// `<name>.local` that phones can reach this computer at over mDNS (Bonjour).
pub fn local_dns_name() -> Option<String> {
    pick_local_dns_name(bonjour_host_name().as_deref(), &gethostname::gethostname().to_string_lossy())
}

/// macOS answers mDNS only for its Bonjour LocalHostName, which can differ
/// from the hostname (a DHCP server may hand out "Mac"), so that name wins.
/// Elsewhere the hostname is the mDNS name.
pub fn pick_local_dns_name(bonjour: Option<&str>, hostname: &str) -> Option<String> {
    bonjour.and_then(tls::local_dns_name).or_else(|| tls::local_dns_name(hostname))
}

/// The LocalHostName from System Settings › General › Sharing › Local hostname
/// (what `scutil --get LocalHostName` prints).
#[cfg(target_os = "macos")]
pub fn bonjour_host_name() -> Option<String> {
    // SAFETY: a NULL store opens a temporary session; the CFString is ours.
    unsafe { macos::take_string(macos::SCDynamicStoreCopyLocalHostName(std::ptr::null())) }
}

#[cfg(not(target_os = "macos"))]
pub fn bonjour_host_name() -> Option<String> {
    None
}

/// The Computer Name from System Settings › General › Sharing (what
/// `scutil --get ComputerName` prints), e.g. "MyMac" or "小明的 MacBook Pro".
#[cfg(target_os = "macos")]
pub fn computer_name() -> Option<String> {
    // SAFETY: as above; the encoding out-parameter may be NULL.
    unsafe { macos::take_string(macos::SCDynamicStoreCopyComputerName(std::ptr::null(), std::ptr::null_mut())) }
}

#[cfg(not(target_os = "macos"))]
pub fn computer_name() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CStr, c_char, c_void};

    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        pub fn SCDynamicStoreCopyLocalHostName(store: *const c_void) -> *const c_void;
        pub fn SCDynamicStoreCopyComputerName(store: *const c_void, encoding: *mut u32) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(s: *const c_void, buf: *mut c_char, len: isize, encoding: u32) -> u8;
        fn CFRelease(cf: *const c_void);
    }
    const UTF8: u32 = 0x0800_0100;

    /// Copies out and releases a CFString from a SystemConfiguration "Copy"
    /// function. Blank names and NULL give `None`.
    ///
    /// # Safety
    /// `s` must be NULL or a CFString the caller owns.
    pub unsafe fn take_string(s: *const c_void) -> Option<String> {
        if s.is_null() {
            return None;
        }
        // Room for a long non-ASCII Computer Name (up to 4 bytes a character).
        let mut buf = [0 as c_char; 1024];
        let ok = unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) } != 0;
        unsafe { CFRelease(s) };
        if !ok {
            return None;
        }
        let text = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().trim().to_string();
        (!text.is_empty()).then_some(text)
    }
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
        assert!(is_virtual("vEthernet (Default Switch)"));
        assert!(!is_virtual("vEthernet (External LAN)"));
        assert!(!is_virtual("en0"));
        assert!(!is_virtual("Wi-Fi"));
    }
}
