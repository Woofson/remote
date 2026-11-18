//! Shared TCP connect for protocols that open their own sockets (SSH, Telnet, VNC,
//! connection test): hostnames, IPv4, and IPv6 with or without brackets.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// `host:port` for display and resolution; bare IPv6 literals get brackets.
pub fn host_port(host: &str, port: u16) -> String {
    let h = host.trim();
    let h = h
        .strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
        .unwrap_or(h);
    if h.contains(':') {
        format!("[{}]:{}", h, port)
    } else {
        format!("{}:{}", h, port)
    }
}

/// Resolve a host (name or address) to all of its socket addresses.
pub fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    let name = host.trim();
    if name.is_empty() {
        return Err("No host given".into());
    }
    let addrs: Vec<SocketAddr> = host_port(name, port)
        .to_socket_addrs()
        .map_err(|e| format!("Cannot resolve host '{}': {}", name, e))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("Cannot resolve host '{}': no addresses found", name));
    }
    Ok(addrs)
}

/// Connect to the first reachable address of `host`, each attempt limited by `timeout`.
pub fn connect_tcp(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let target = host_port(host, port);
    let mut last_err = None;
    for addr in resolve(host, port)? {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => return Ok(stream),
            Err(e) => last_err = Some(e),
        }
    }
    let e = last_err.expect("resolve returns at least one address");
    Err(match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            format!("Connection to {} timed out", target)
        }
        _ => format!("Cannot connect to {}: {}", target, e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_hosts() {
        assert_eq!(host_port("10.0.0.1", 22), "10.0.0.1:22");
        assert_eq!(host_port(" meteorite.lan ", 22), "meteorite.lan:22");
        assert_eq!(host_port("fd00::5", 22), "[fd00::5]:22");
        assert_eq!(host_port("[fd00::5]", 22), "[fd00::5]:22");
    }

    #[test]
    fn resolves_names_and_literals() {
        assert_eq!(resolve("127.0.0.1", 22).unwrap()[0].port(), 22);
        assert!(resolve("::1", 22).unwrap()[0].is_ipv6());
        assert!(resolve("[::1]", 22).unwrap()[0].is_ipv6());
        assert!(!resolve("localhost", 22).unwrap().is_empty());
        assert!(resolve("", 22).is_err());
        assert!(resolve("no-such-host.invalid", 22).is_err());
    }

    #[test]
    fn reports_refused_connections() {
        // Port 1 on loopback is closed on any normal machine.
        let err = connect_tcp("127.0.0.1", 1, Duration::from_secs(2)).unwrap_err();
        assert!(err.starts_with("Cannot connect to 127.0.0.1:1"), "{}", err);
    }
}
