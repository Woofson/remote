//! Wake-on-LAN (#35): wake a sleeping or switched-off computer before connecting to it.

use serde_json::Value;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

/// Wake-on-LAN settings of a connection (stored in its settings).
#[derive(Debug, Clone, PartialEq)]
pub struct WakeSettings {
    pub mac: [u8; 6],
    pub broadcast: Ipv4Addr,
    pub port: u16,
    /// Wake automatically when someone connects and the machine does not answer.
    pub on_connect: bool,
    /// How long to wait for the machine to come up, in seconds.
    pub wait_secs: u64,
}

pub const DEFAULT_WAIT_SECS: u64 = 120;

impl WakeSettings {
    /// From a connection's settings JSON; `None` when no (valid) MAC address is set.
    pub fn from_settings(settings_json: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(settings_json).ok()?;
        let mac = parse_mac(v.get("wol_mac")?.as_str()?)?;
        let broadcast = v
            .get("wol_broadcast")
            .and_then(|b| b.as_str())
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .and_then(|b| b.parse().ok())
            .unwrap_or(Ipv4Addr::BROADCAST);
        let port = v.get("wol_port").and_then(|p| p.as_u64()).filter(|p| (1..=65535).contains(p)).unwrap_or(9) as u16;
        let on_connect = v.get("wol_on_connect").and_then(|b| b.as_bool()).unwrap_or(false);
        let wait_secs = v.get("wol_wait").and_then(|w| w.as_u64()).unwrap_or(DEFAULT_WAIT_SECS).clamp(10, 600);
        Some(Self { mac, broadcast, port, on_connect, wait_secs })
    }

    pub fn mac_text(&self) -> String {
        self.mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
    }
}

/// `aa:bb:cc:dd:ee:ff`, `aa-bb-cc-dd-ee-ff`, `aabb.ccdd.eeff` or `aabbccddeeff`.
pub fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let hex: String = text.chars().filter(|c| !matches!(c, ':' | '-' | '.' | ' ')).collect();
    if hex.len() != 12 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, b) in mac.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(mac)
}

/// The magic packet: six 0xFF bytes, then the MAC address sixteen times.
pub fn magic_packet(mac: &[u8; 6]) -> [u8; 102] {
    let mut p = [0xFFu8; 102];
    for i in 0..16 {
        p[6 + i * 6..12 + i * 6].copy_from_slice(mac);
    }
    p
}

/// Send the magic packet (a few times, as UDP may drop one).
pub fn send_wake(w: &WakeSettings) -> std::io::Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_broadcast(true)?;
    let packet = magic_packet(&w.mac);
    let target = SocketAddr::from((w.broadcast, w.port));
    for _ in 0..3 {
        socket.send_to(&packet, target)?;
    }
    Ok(())
}

/// Does the machine answer on its connection port?
pub async fn reachable(host: &str, port: u16, timeout: Duration) -> bool {
    matches!(tokio::time::timeout(timeout, tokio::net::TcpStream::connect((host, port))).await, Ok(Ok(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_formats() {
        let want = Some([0xaa, 0xbb, 0xcc, 0x01, 0x02, 0x03]);
        assert_eq!(parse_mac("aa:bb:cc:01:02:03"), want);
        assert_eq!(parse_mac("AA-BB-CC-01-02-03"), want);
        assert_eq!(parse_mac("aabb.cc01.0203"), want);
        assert_eq!(parse_mac("aabbcc010203"), want);
        assert_eq!(parse_mac("aa:bb:cc:01:02"), None);
        assert_eq!(parse_mac("zz:bb:cc:01:02:03"), None);
    }

    #[test]
    fn packet_and_settings() {
        let mac = [1, 2, 3, 4, 5, 6];
        let p = magic_packet(&mac);
        assert_eq!(&p[..6], &[0xFF; 6]);
        assert_eq!(&p[6..12], &mac);
        assert_eq!(&p[96..102], &mac);
        let w = WakeSettings::from_settings(r#"{"wol_mac":"01:02:03:04:05:06","wol_on_connect":true,"wol_wait":5}"#).unwrap();
        assert_eq!(w.broadcast, Ipv4Addr::BROADCAST);
        assert_eq!(w.port, 9);
        assert_eq!(w.wait_secs, 10);
        assert!(w.on_connect);
        assert!(WakeSettings::from_settings(r#"{"wol_mac":""}"#).is_none());
        assert!(WakeSettings::from_settings("{}").is_none());
    }
}
