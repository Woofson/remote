use axum::extract::ws::{Message, WebSocket};
use flate2::{Decompress, FlushDecompress};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

pub struct VncConnectionParams {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VncServerInfo {
    pub width: u16,
    pub height: u16,
    pub name: String,
}

// ================= VNC Stream Abstraction (Plain TCP + VeNCrypt TLS) =================

pub enum VncStream {
    Plain(TcpStream),
    #[cfg(not(target_os = "windows"))]
    Tls(openssl::ssl::SslStream<TcpStream>),
}

impl VncStream {
    fn raw_fd(&self) -> i32 {
        match self {
            VncStream::Plain(s) => s.as_raw_fd(),
            #[cfg(not(target_os = "windows"))]
            VncStream::Tls(s) => s.get_ref().as_raw_fd(),
        }
    }

    fn has_pending_or_readable(&self, timeout_ms: i32) -> Result<bool, std::io::Error> {
        #[cfg(not(target_os = "windows"))]
        if let VncStream::Tls(s) = self {
            if s.ssl().pending() > 0 {
                return Ok(true);
            }
        }

        let fd = self.raw_fd();
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ret = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                return Ok(false);
            }
            return Err(err);
        }
        if ret == 0 {
            return Ok(false);
        }
        if (pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0
            && (pfd.revents & libc::POLLIN) == 0
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "VNC socket closed by peer",
            ));
        }
        Ok((pfd.revents & libc::POLLIN) != 0)
    }

    fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        match self {
            VncStream::Plain(s) => s.set_read_timeout(dur),
            #[cfg(not(target_os = "windows"))]
            VncStream::Tls(s) => s.get_ref().set_read_timeout(dur),
        }
    }
}

impl Read for VncStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            VncStream::Plain(s) => s.read(buf),
            #[cfg(not(target_os = "windows"))]
            VncStream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for VncStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            VncStream::Plain(s) => s.write(buf),
            #[cfg(not(target_os = "windows"))]
            VncStream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            VncStream::Plain(s) => s.flush(),
            #[cfg(not(target_os = "windows"))]
            VncStream::Tls(s) => s.flush(),
        }
    }
}

// ================= Standard FIPS 46-3 DES-ECB for VNC Authentication =================

const IP_TABLE: [u8; 64] = [
    58, 50, 42, 34, 26, 18, 10, 2, 60, 52, 44, 36, 28, 20, 12, 4,
    62, 54, 46, 38, 30, 22, 14, 6, 64, 56, 48, 40, 32, 24, 16, 8,
    57, 49, 41, 33, 25, 17, 9, 1, 59, 51, 43, 35, 27, 19, 11, 3,
    61, 53, 45, 37, 29, 21, 13, 5, 63, 55, 47, 39, 31, 23, 15, 7,
];

const FP_TABLE: [u8; 64] = [
    40, 8, 48, 16, 56, 24, 64, 32, 39, 7, 47, 15, 55, 23, 63, 31,
    38, 6, 46, 14, 54, 22, 62, 30, 37, 5, 45, 13, 53, 21, 61, 29,
    36, 4, 44, 12, 52, 20, 60, 28, 35, 3, 43, 11, 51, 19, 59, 27,
    34, 2, 42, 10, 50, 18, 58, 26, 33, 1, 41, 9, 49, 17, 57, 25,
];

const E_TABLE: [u8; 48] = [
    32, 1, 2, 3, 4, 5, 4, 5, 6, 7, 8, 9,
    8, 9, 10, 11, 12, 13, 12, 13, 14, 15, 16, 17,
    16, 17, 18, 19, 20, 21, 20, 21, 22, 23, 24, 25,
    24, 25, 26, 27, 28, 29, 28, 29, 30, 31, 32, 1,
];

const P_TABLE: [u8; 32] = [
    16, 7, 20, 21, 29, 12, 28, 17, 1, 15, 23, 26, 5, 18, 31, 10,
    2, 8, 24, 14, 32, 27, 3, 9, 19, 13, 30, 6, 22, 11, 4, 25,
];

const PC1_TABLE: [u8; 56] = [
    57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18,
    10, 2, 59, 51, 43, 35, 27, 19, 11, 3, 60, 52, 44, 36,
    63, 55, 47, 39, 31, 23, 15, 7, 62, 54, 46, 38, 30, 22,
    14, 6, 61, 53, 45, 37, 29, 21, 13, 5, 28, 20, 12, 4,
];

const PC2_TABLE: [u8; 48] = [
    14, 17, 11, 24, 1, 5, 3, 28, 15, 6, 21, 10,
    23, 19, 12, 4, 26, 8, 16, 7, 27, 20, 13, 2,
    41, 52, 31, 37, 47, 55, 30, 40, 51, 45, 33, 48,
    44, 49, 39, 56, 34, 53, 46, 42, 50, 36, 29, 32,
];

const KEY_SHIFTS: [u8; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];

const S_BOXES: [[[u8; 16]; 4]; 8] = [
    // S1
    [
        [14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7],
        [0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12, 11, 9, 5, 3, 8],
        [4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0],
        [15, 12, 8, 2, 4, 9, 1, 7, 5, 11, 3, 14, 10, 0, 6, 13],
    ],
    // S2
    [
        [15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10],
        [3, 13, 4, 7, 15, 2, 8, 14, 12, 0, 1, 10, 6, 9, 11, 5],
        [0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15],
        [13, 8, 10, 1, 3, 15, 4, 2, 11, 6, 7, 12, 0, 5, 14, 9],
    ],
    // S3
    [
        [10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8],
        [13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5, 14, 12, 11, 15, 1],
        [13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7],
        [1, 10, 13, 0, 6, 9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12],
    ],
    // S4
    [
        [7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15],
        [13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2, 12, 1, 10, 14, 9],
        [10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4],
        [3, 15, 0, 6, 10, 1, 13, 8, 9, 4, 5, 11, 12, 7, 2, 14],
    ],
    // S5
    [
        [2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9],
        [14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15, 10, 3, 9, 8, 6],
        [4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14],
        [11, 8, 12, 7, 1, 14, 2, 13, 6, 15, 0, 9, 10, 4, 5, 3],
    ],
    // S6
    [
        [12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11],
        [10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13, 14, 0, 11, 3, 8],
        [9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6],
        [4, 3, 2, 12, 9, 5, 15, 10, 11, 14, 1, 7, 6, 0, 8, 13],
    ],
    // S7
    [
        [4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1],
        [13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5, 12, 2, 15, 8, 6],
        [1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2],
        [6, 11, 13, 8, 1, 4, 10, 7, 9, 5, 0, 15, 14, 2, 3, 12],
    ],
    // S8
    [
        [13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7],
        [1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6, 11, 0, 14, 9, 2],
        [7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8],
        [2, 1, 14, 7, 4, 10, 8, 13, 15, 12, 9, 0, 3, 5, 6, 11],
    ],
];

#[inline]
fn permute(input: u64, table: &[u8], in_bits: u8) -> u64 {
    let mut out = 0u64;
    for (i, &pos) in table.iter().enumerate() {
        let bit = (input >> (in_bits - pos)) & 1;
        out |= bit << (table.len() - 1 - i);
    }
    out
}

pub fn des_ecb_encrypt_block(key_bytes: &[u8; 8], block_bytes: &[u8; 8]) -> [u8; 8] {
    let key = u64::from_be_bytes(*key_bytes);
    let block = u64::from_be_bytes(*block_bytes);

    // Key schedule (PC1 -> 56 bits -> C0, D0 of 28 bits each)
    let permuted_key = permute(key, &PC1_TABLE, 64);
    let mut c = ((permuted_key >> 28) & 0x0fff_ffff) as u32;
    let mut d = (permuted_key & 0x0fff_ffff) as u32;

    let mut subkeys = [0u64; 16];
    for i in 0..16 {
        let shift = KEY_SHIFTS[i];
        c = ((c << shift) | (c >> (28 - shift))) & 0x0fff_ffff;
        d = ((d << shift) | (d >> (28 - shift))) & 0x0fff_ffff;
        let cd = ((c as u64) << 28) | (d as u64);
        subkeys[i] = permute(cd, &PC2_TABLE, 56);
    }

    // Initial Permutation (IP)
    let ip = permute(block, &IP_TABLE, 64);
    let mut left = ((ip >> 32) & 0xffff_ffff) as u32;
    let mut right = (ip & 0xffff_ffff) as u32;

    // 16 Feistel rounds
    for subkey in subkeys {
        let expanded = permute(right as u64, &E_TABLE, 32) ^ subkey;
        let mut s_out = 0u32;
        for s_idx in 0..8 {
            let six_bits = ((expanded >> (42 - s_idx * 6)) & 0x3f) as u8;
            let row = (((six_bits & 0x20) >> 4) | (six_bits & 0x01)) as usize;
            let col = ((six_bits >> 1) & 0x0f) as usize;
            let val = S_BOXES[s_idx][row][col] as u32;
            s_out = (s_out << 4) | val;
        }
        let f_val = permute(s_out as u64, &P_TABLE, 32) as u32;
        let next_right = left ^ f_val;
        left = right;
        right = next_right;
    }

    // Swap (R16, L16) and Final Permutation (FP)
    let preoutput = ((right as u64) << 32) | (left as u64);
    let fp = permute(preoutput, &FP_TABLE, 64);
    fp.to_be_bytes()
}

pub fn vnc_des_encrypt(challenge: &[u8; 16], password: &str) -> [u8; 16] {
    let mut key = [0u8; 8];
    let pw_bytes = password.as_bytes();
    for i in 0..8 {
        if i < pw_bytes.len() {
            // Mirror bits in byte for RFB / VNC standard (RFC 6143 Section 7.2.2)
            key[i] = pw_bytes[i].reverse_bits();
        }
    }

    let mut block1 = [0u8; 8];
    let mut block2 = [0u8; 8];
    block1.copy_from_slice(&challenge[0..8]);
    block2.copy_from_slice(&challenge[8..16]);

    let enc1 = des_ecb_encrypt_block(&key, &block1);
    let enc2 = des_ecb_encrypt_block(&key, &block2);

    let mut response = [0u8; 16];
    response[0..8].copy_from_slice(&enc1);
    response[8..16].copy_from_slice(&enc2);
    response
}

// ================= VNC Authentication Helpers =================

fn read_server_reason<R: Read>(stream: &mut R) -> String {
    let mut len_buf = [0u8; 4];
    if stream.read_exact(&mut len_buf).is_ok() {
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > 0 && len <= 65536 {
            let mut reason_buf = vec![0u8; len];
            if stream.read_exact(&mut reason_buf).is_ok() {
                return String::from_utf8_lossy(&reason_buf).trim().to_string();
            }
        }
    }
    String::new()
}

fn perform_vnc_des_auth<S: Read + Write>(stream: &mut S, password: &str) -> Result<(), String> {
    let mut challenge = [0u8; 16];
    stream
        .read_exact(&mut challenge)
        .map_err(|e| format!("Failed to read VNC 16-byte challenge: {}", e))?;

    let response = vnc_des_encrypt(&challenge, password);
    stream
        .write_all(&response)
        .map_err(|e| format!("Failed to send VNC auth response: {}", e))?;
    stream.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn perform_vencrypt_plain_auth<S: Read + Write>(
    stream: &mut S,
    username: &str,
    password: &str,
) -> Result<(), String> {
    let u_bytes = username.as_bytes();
    let p_bytes = password.as_bytes();
    let mut pkt = Vec::with_capacity(8 + u_bytes.len() + p_bytes.len());
    pkt.extend_from_slice(&(u_bytes.len() as u32).to_be_bytes());
    pkt.extend_from_slice(&(p_bytes.len() as u32).to_be_bytes());
    pkt.extend_from_slice(u_bytes);
    pkt.extend_from_slice(p_bytes);
    stream
        .write_all(&pkt)
        .map_err(|e| format!("Failed to send VeNCrypt Plain credentials: {}", e))?;
    stream.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn check_security_result<R: Read>(stream: &mut R, is_rfb_38: bool) -> Result<(), String> {
    let mut auth_result_buf = [0u8; 4];
    stream
        .read_exact(&mut auth_result_buf)
        .map_err(|e| format!("Failed to read VNC security result: {}", e))?;
    let auth_res = u32::from_be_bytes(auth_result_buf);
    if auth_res != 0 {
        let reason = if is_rfb_38 {
            read_server_reason(stream)
        } else {
            String::new()
        };
        if reason.is_empty() {
            return Err("VNC authentication failed (invalid password or rejected by server)".into());
        } else {
            return Err(format!("VNC authentication failed: {}", reason));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn upgrade_stream_to_tls(
    tcp_stream: TcpStream,
    host: &str,
    anonymous_dh: bool,
) -> Result<VncStream, String> {
    use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};

    let mut builder = SslConnector::builder(SslMethod::tls())
        .map_err(|e| format!("Failed to initialize OpenSSL connector: {}", e))?;

    // VNC servers (TigerVNC, WayVNC, QEMU, Proxmox) commonly use self-signed X509 or ADH
    builder.set_verify(SslVerifyMode::NONE);
    if anonymous_dh {
        let _ = builder.set_cipher_list("ALL:ADH:AECDH:@SECLEVEL=0");
    } else {
        let _ = builder.set_cipher_list("ALL:@SECLEVEL=0");
    }

    let connector = builder.build();
    let mut ssl_cfg = connector
        .configure()
        .map_err(|e| format!("Failed to configure TLS for VNC: {}", e))?;
    ssl_cfg.set_verify_hostname(false);
    ssl_cfg.set_use_server_name_indication(!anonymous_dh);

    let tls_stream = ssl_cfg
        .connect(host, tcp_stream)
        .map_err(|e| format!("VNC TLS handshake failed: {}", e))?;

    Ok(VncStream::Tls(tls_stream))
}

pub fn connect_and_handshake_vnc(
    params: &VncConnectionParams,
) -> Result<(VncStream, VncServerInfo), String> {
    let addr_str = format!("{}:{}", params.host.trim(), params.port);
    let sock_addr = addr_str
        .to_socket_addrs()
        .map_err(|e| format!("Unable to resolve VNC host '{}': {}", addr_str, e))?
        .next()
        .ok_or_else(|| format!("No IP addresses found for VNC host '{}'", addr_str))?;

    let mut tcp_stream = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(10))
        .map_err(|e| format!("Failed to connect to VNC server at {}: {}", addr_str, e))?;

    let _ = tcp_stream.set_nodelay(true);
    tcp_stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|e| e.to_string())?;

    // 1. ProtocolVersion Handshake (12 bytes: "RFB xxx.yyy\n")
    let mut version_buf = [0u8; 12];
    tcp_stream
        .read_exact(&mut version_buf)
        .map_err(|e| format!("Failed to read VNC version banner: {}", e))?;

    let server_ver = String::from_utf8_lossy(&version_buf);
    info!("VNC Gateway: Server Version banner: {}", server_ver.trim());

    if !server_ver.starts_with("RFB ") {
        return Err(format!(
            "Target {}:{} is not a valid VNC/RFB server (banner: {:?})",
            params.host,
            params.port,
            server_ver.trim()
        ));
    }

    // Parse minor version to handle RFB 3.3, 3.7, and 3.8+ (including 3.889 Apple/Tight)
    let minor_ver: u32 = server_ver
        .get(8..11)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);

    let client_minor = if minor_ver <= 6 {
        3
    } else if minor_ver == 7 {
        7
    } else {
        8
    };
    let is_rfb_38 = client_minor >= 8;

    let client_ver_str = format!("RFB 003.{:03}\n", client_minor);
    tcp_stream
        .write_all(client_ver_str.as_bytes())
        .map_err(|e| format!("Failed to send RFB version: {}", e))?;
    tcp_stream.flush().map_err(|e| e.to_string())?;

    let password = params.password.as_deref().unwrap_or("");
    let username = params.username.as_deref().unwrap_or("");

    // 2. Security Negotiation
    let mut stream = if client_minor == 3 {
        // RFB 3.3: Server sends a u32 security type directly
        let mut sec_buf = [0u8; 4];
        tcp_stream
            .read_exact(&mut sec_buf)
            .map_err(|e| format!("Failed to read RFB 3.3 security type: {}", e))?;
        let sec_type = u32::from_be_bytes(sec_buf);
        match sec_type {
            0 => {
                let reason = read_server_reason(&mut tcp_stream);
                return Err(format!("VNC connection rejected by server: {}", reason));
            }
            1 => {
                // None: in RFB 3.3, no SecurityResult follows
                VncStream::Plain(tcp_stream)
            }
            2 => {
                perform_vnc_des_auth(&mut tcp_stream, password)?;
                check_security_result(&mut tcp_stream, false)?;
                VncStream::Plain(tcp_stream)
            }
            other => {
                return Err(format!(
                    "Unsupported RFB 3.3 security type {} required by server",
                    other
                ));
            }
        }
    } else {
        // RFB 3.7 / 3.8: Server sends u8 count + array of security types
        let mut num_types_buf = [0u8; 1];
        tcp_stream
            .read_exact(&mut num_types_buf)
            .map_err(|e| format!("Failed to read security types count: {}", e))?;
        let num_types = num_types_buf[0] as usize;

        if num_types == 0 {
            let reason = read_server_reason(&mut tcp_stream);
            return Err(format!("VNC connection rejected: {}", reason));
        }

        let mut sec_types = vec![0u8; num_types];
        tcp_stream
            .read_exact(&mut sec_types)
            .map_err(|e| format!("Failed to read security types: {}", e))?;

        info!("VNC Gateway: Server offered security types: {:?}", sec_types);

        // Selection priority:
        // - If password is provided and type 2 (VncAuth) is offered, prefer 2
        // - Else if type 1 (None) is offered, select 1
        // - Else if type 2 (VncAuth) is offered, select 2
        // - Else if type 19 (VeNCrypt - TigerVNC/WayVNC/QEMU) is offered, select 19
        // - Else if type 16 (Tight) is offered, select 16
        let selected_type = if !password.is_empty() && sec_types.contains(&2) {
            2u8
        } else if sec_types.contains(&1) {
            1u8
        } else if sec_types.contains(&2) {
            2u8
        } else if sec_types.contains(&19) {
            19u8
        } else if sec_types.contains(&16) {
            16u8
        } else {
            return Err(format!(
                "VNC server requires unsupported security types {:?}. Supported: None (1), VNC Password (2), Tight (16), VeNCrypt/TLS (19).",
                sec_types
            ));
        };

        tcp_stream
            .write_all(&[selected_type])
            .map_err(|e| format!("Failed to send selected security type: {}", e))?;
        tcp_stream.flush().map_err(|e| e.to_string())?;

        match selected_type {
            1 => {
                // None: only RFB 3.8 sends a 4-byte SecurityResult after None
                if is_rfb_38 {
                    check_security_result(&mut tcp_stream, true)?;
                }
                VncStream::Plain(tcp_stream)
            }
            2 => {
                perform_vnc_des_auth(&mut tcp_stream, password)?;
                check_security_result(&mut tcp_stream, is_rfb_38)?;
                VncStream::Plain(tcp_stream)
            }
            16 => {
                // TightVNC Security Type (16)
                let mut n_tunnels_buf = [0u8; 4];
                tcp_stream
                    .read_exact(&mut n_tunnels_buf)
                    .map_err(|e| format!("TightVNC: failed to read tunnels count: {}", e))?;
                let n_tunnels = u32::from_be_bytes(n_tunnels_buf) as usize;
                if n_tunnels > 0 {
                    let mut tunnel_caps = vec![0u8; n_tunnels * 16];
                    tcp_stream
                        .read_exact(&mut tunnel_caps)
                        .map_err(|e| format!("TightVNC: failed to read tunnel caps: {}", e))?;
                    // Select NOTUNNEL (0)
                    tcp_stream
                        .write_all(&0u32.to_be_bytes())
                        .map_err(|e| e.to_string())?;
                    tcp_stream.flush().map_err(|e| e.to_string())?;
                }

                let mut n_auth_buf = [0u8; 4];
                tcp_stream
                    .read_exact(&mut n_auth_buf)
                    .map_err(|e| format!("TightVNC: failed to read auth count: {}", e))?;
                let n_auth = u32::from_be_bytes(n_auth_buf) as usize;
                if n_auth == 0 {
                    if is_rfb_38 {
                        check_security_result(&mut tcp_stream, true)?;
                    }
                    VncStream::Plain(tcp_stream)
                } else {
                    let mut auth_codes = Vec::with_capacity(n_auth);
                    for _ in 0..n_auth {
                        let mut cap = [0u8; 16];
                        tcp_stream
                            .read_exact(&mut cap)
                            .map_err(|e| format!("TightVNC: failed to read auth cap: {}", e))?;
                        let code = u32::from_be_bytes([cap[0], cap[1], cap[2], cap[3]]);
                        auth_codes.push(code);
                    }
                    let chosen_code = if !password.is_empty() && auth_codes.contains(&2) {
                        2u32
                    } else if auth_codes.contains(&1) {
                        1u32
                    } else if auth_codes.contains(&2) {
                        2u32
                    } else {
                        return Err(format!(
                            "TightVNC server requires unsupported auth codes {:?}",
                            auth_codes
                        ));
                    };
                    tcp_stream
                        .write_all(&chosen_code.to_be_bytes())
                        .map_err(|e| e.to_string())?;
                    tcp_stream.flush().map_err(|e| e.to_string())?;

                    if chosen_code == 2 {
                        perform_vnc_des_auth(&mut tcp_stream, password)?;
                    }
                    check_security_result(&mut tcp_stream, is_rfb_38)?;
                    VncStream::Plain(tcp_stream)
                }
            }
            19 => {
                // VeNCrypt (19) - TigerVNC / WayVNC / QEMU
                let mut ver_buf = [0u8; 2];
                tcp_stream
                    .read_exact(&mut ver_buf)
                    .map_err(|e| format!("VeNCrypt: failed to read server version: {}", e))?;
                // Reply with VeNCrypt 0.2
                tcp_stream
                    .write_all(&[0, 2])
                    .map_err(|e| format!("VeNCrypt: failed to send version 0.2: {}", e))?;
                tcp_stream.flush().map_err(|e| e.to_string())?;

                let mut ack_buf = [0u8; 1];
                tcp_stream
                    .read_exact(&mut ack_buf)
                    .map_err(|e| format!("VeNCrypt: failed to read version ack: {}", e))?;
                if ack_buf[0] != 0 {
                    return Err("VeNCrypt version 0.2 rejected by VNC server".into());
                }

                let mut n_sub_buf = [0u8; 1];
                tcp_stream
                    .read_exact(&mut n_sub_buf)
                    .map_err(|e| format!("VeNCrypt: failed to read subtypes count: {}", e))?;
                let n_sub = n_sub_buf[0] as usize;
                if n_sub == 0 {
                    return Err("VeNCrypt server offered 0 subtypes".into());
                }

                let mut subtypes = Vec::with_capacity(n_sub);
                for _ in 0..n_sub {
                    let mut sb = [0u8; 4];
                    tcp_stream
                        .read_exact(&mut sb)
                        .map_err(|e| format!("VeNCrypt: failed to read subtype: {}", e))?;
                    subtypes.push(u32::from_be_bytes(sb));
                }

                info!("VNC Gateway: VeNCrypt offered subtypes: {:?}", subtypes);

                // Subtypes:
                // 261 = X509Vnc, 258 = TLSVnc, 2 = VncAuth
                // 262 = X509Plain, 259 = TLSPlain, 256 = Plain
                // 260 = X509None, 257 = TLSNone, 1 = None
                let preferred_order: &[u32] = if !password.is_empty() && !username.is_empty() {
                    &[261, 258, 262, 259, 2, 256, 260, 257, 1]
                } else if !password.is_empty() {
                    &[261, 258, 2, 262, 259, 256, 260, 257, 1]
                } else {
                    &[260, 257, 1, 261, 258, 2, 262, 259, 256]
                };

                let chosen_sub = preferred_order
                    .iter()
                    .copied()
                    .find(|s| subtypes.contains(s))
                    .ok_or_else(|| {
                        format!(
                            "VeNCrypt server requires unsupported subtypes {:?}",
                            subtypes
                        )
                    })?;

                tcp_stream
                    .write_all(&chosen_sub.to_be_bytes())
                    .map_err(|e| format!("VeNCrypt: failed to send chosen subtype: {}", e))?;
                tcp_stream.flush().map_err(|e| e.to_string())?;

                // Server replies with 1-byte acceptance (1 = accepted)
                let mut sub_ack = [0u8; 1];
                tcp_stream
                    .read_exact(&mut sub_ack)
                    .map_err(|e| format!("VeNCrypt: failed to read subtype ack: {}", e))?;
                if sub_ack[0] != 1 {
                    return Err(format!(
                        "VeNCrypt server rejected subtype {} (ack={})",
                        chosen_sub, sub_ack[0]
                    ));
                }

                match chosen_sub {
                    1 => {
                        check_security_result(&mut tcp_stream, is_rfb_38)?;
                        VncStream::Plain(tcp_stream)
                    }
                    2 => {
                        perform_vnc_des_auth(&mut tcp_stream, password)?;
                        check_security_result(&mut tcp_stream, is_rfb_38)?;
                        VncStream::Plain(tcp_stream)
                    }
                    256 => {
                        perform_vencrypt_plain_auth(&mut tcp_stream, username, password)?;
                        check_security_result(&mut tcp_stream, is_rfb_38)?;
                        VncStream::Plain(tcp_stream)
                    }
                    #[cfg(not(target_os = "windows"))]
                    257 | 258 | 259 | 260 | 261 | 262 => {
                        let is_adh = matches!(chosen_sub, 257 | 258 | 259);
                        let mut tls_stream =
                            upgrade_stream_to_tls(tcp_stream, &params.host, is_adh)?;
                        match chosen_sub {
                            257 | 260 => {
                                // TLSNone / X509None
                            }
                            258 | 261 => {
                                // TLSVnc / X509Vnc
                                perform_vnc_des_auth(&mut tls_stream, password)?;
                            }
                            259 | 262 => {
                                // TLSPlain / X509Plain
                                perform_vencrypt_plain_auth(&mut tls_stream, username, password)?;
                            }
                            _ => {}
                        }
                        check_security_result(&mut tls_stream, is_rfb_38)?;
                        tls_stream
                    }
                    other => {
                        return Err(format!("Unsupported VeNCrypt subtype {}", other));
                    }
                }
            }
            other => {
                return Err(format!("Unsupported VNC security type {}", other));
            }
        }
    };

    // 3. ClientInit (Shared desktop = 1)
    stream
        .write_all(&[1])
        .map_err(|e| format!("Failed to send VNC ClientInit: {}", e))?;
    stream.flush().map_err(|e| e.to_string())?;

    // 4. ServerInit (24 bytes + desktop name)
    let mut server_init_buf = [0u8; 24];
    stream
        .read_exact(&mut server_init_buf)
        .map_err(|e| format!("Failed to read VNC ServerInit: {}", e))?;

    let width = u16::from_be_bytes([server_init_buf[0], server_init_buf[1]]);
    let height = u16::from_be_bytes([server_init_buf[2], server_init_buf[3]]);
    let name_len = u32::from_be_bytes([
        server_init_buf[20],
        server_init_buf[21],
        server_init_buf[22],
        server_init_buf[23],
    ]) as usize;

    let name = if name_len > 0 && name_len <= 65536 {
        let mut name_buf = vec![0u8; name_len];
        stream
            .read_exact(&mut name_buf)
            .map_err(|e| format!("Failed to read VNC desktop name: {}", e))?;
        String::from_utf8_lossy(&name_buf).to_string()
    } else {
        "VNC Desktop".to_string()
    };

    info!(
        "VNC Gateway: Connected to Desktop '{}' ({}x{})",
        name, width, height
    );

    // 5. SetPixelFormat (32-bit TrueColor, little-endian, R=0..7, G=8..15, B=16..23)
    let mut pf_msg = [0u8; 20];
    pf_msg[0] = 0; // SetPixelFormat
    pf_msg[4] = 32; // bits-per-pixel
    pf_msg[5] = 24; // depth
    pf_msg[6] = 0; // big-endian-flag (0 = Little Endian)
    pf_msg[7] = 1; // true-colour-flag
    pf_msg[8..10].copy_from_slice(&255u16.to_be_bytes()); // red-max
    pf_msg[10..12].copy_from_slice(&255u16.to_be_bytes()); // green-max
    pf_msg[12..14].copy_from_slice(&255u16.to_be_bytes()); // blue-max
    pf_msg[14] = 0; // red-shift
    pf_msg[15] = 8; // green-shift
    pf_msg[16] = 16; // blue-shift
    stream.write_all(&pf_msg).map_err(|e| e.to_string())?;

    // 6. SetEncodings:
    // CopyRect (1), ZRLE (16), Zlib (6), Hextile (5), Raw (0),
    // ExtendedDesktopSize (-308), DesktopSize (-223), LastRect (-224), Cursor (-239)
    let encodings: [i32; 9] = [1, 16, 6, 5, 0, -308, -223, -224, -239];
    let mut enc_msg = Vec::with_capacity(4 + encodings.len() * 4);
    enc_msg.push(2); // SetEncodings
    enc_msg.push(0); // padding
    enc_msg.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
    for enc in encodings {
        enc_msg.extend_from_slice(&enc.to_be_bytes());
    }
    stream.write_all(&enc_msg).map_err(|e| e.to_string())?;

    // 7. Initial FramebufferUpdateRequest (Incremental = 0)
    let fbur = build_fbur_packet(0, 0, 0, width, height);
    stream.write_all(&fbur).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    Ok((
        stream,
        VncServerInfo {
            width,
            height,
            name,
        },
    ))
}

#[inline]
fn build_fbur_packet(incremental: u8, x: u16, y: u16, w: u16, h: u16) -> [u8; 10] {
    let mut fbur = [0u8; 10];
    fbur[0] = 3; // FramebufferUpdateRequest
    fbur[1] = incremental;
    fbur[2..4].copy_from_slice(&x.to_be_bytes());
    fbur[4..6].copy_from_slice(&y.to_be_bytes());
    fbur[6..8].copy_from_slice(&w.to_be_bytes());
    fbur[8..10].copy_from_slice(&h.to_be_bytes());
    fbur
}

#[inline]
fn build_key_event(down: u8, keysym: u32) -> [u8; 8] {
    let mut ke = [0u8; 8];
    ke[0] = 4; // KeyEvent
    ke[1] = down;
    ke[4..8].copy_from_slice(&keysym.to_be_bytes());
    ke
}

/// Map browser PS/2 scancode + extended flag + unicode char to X11 keysym (RFC 6143 Section 7.5.4)
pub fn map_browser_key_to_keysym(scancode: u8, extended: u8, unicode: u16) -> Option<u32> {
    // 1. Check non-printable / control / navigation / modifier keys by hardware scancode
    match (scancode, extended) {
        (0x01, 0) => return Some(0xff1b), // Escape
        (0x0E, 0) => return Some(0xff08), // BackSpace
        (0x0F, 0) => return Some(0xff09), // Tab
        (0x1C, 0) => return Some(0xff0d), // Return / Enter
        (0x1C, 1) => return Some(0xff8d), // KP_Enter
        (0x1D, 0) => return Some(0xffe3), // Control_L
        (0x1D, 1) => return Some(0xffe4), // Control_R
        (0x2A, 0) => return Some(0xffe1), // Shift_L
        (0x36, 0) => return Some(0xffe2), // Shift_R
        (0x38, 0) => return Some(0xffe9), // Alt_L
        (0x38, 1) => return Some(0xffea), // Alt_R / ISO_Level3_Shift
        (0x5B, 1) => return Some(0xffeb), // Super_L / Meta_L
        (0x5C, 1) => return Some(0xffec), // Super_R / Meta_R
        (0x5D, 1) => return Some(0xff67), // Menu
        (0x3A, 0) => return Some(0xffe5), // Caps_Lock
        (0x45, 0) | (0x45, 1) => return Some(0xff7f), // Num_Lock
        (0x46, 0) => return Some(0xff14), // Scroll_Lock
        (0x3B..=0x44, 0) => return Some(0xffbe + (scancode - 0x3B) as u32), // F1..F10
        (0x57, 0) => return Some(0xffc8), // F11
        (0x58, 0) => return Some(0xffc9), // F12
        (0x47, 1) => return Some(0xff50), // Home
        (0x48, 1) => return Some(0xff52), // Up
        (0x49, 1) => return Some(0xff55), // Page_Up
        (0x4B, 1) => return Some(0xff51), // Left
        (0x4D, 1) => return Some(0xff53), // Right
        (0x4F, 1) => return Some(0xff57), // End
        (0x50, 1) => return Some(0xff54), // Down
        (0x51, 1) => return Some(0xff56), // Page_Down
        (0x52, 1) => return Some(0xff63), // Insert
        (0x53, 1) => return Some(0xffff), // Delete
        _ => {}
    }

    // 2. Printable Unicode character (ASCII, Latin-1 æøå, or general Unicode)
    if unicode >= 0x0020 && unicode <= 0x00ff {
        return Some(unicode as u32);
    } else if unicode >= 0x0100 {
        return Some(0x0100_0000 | (unicode as u32));
    }

    // 3. Fallback for scancode when unicode == 0 (e.g. keyup or Ctrl+letter combinations)
    match scancode {
        0x02..=0x0A => Some((b'1' + (scancode - 0x02)) as u32),
        0x0B => Some(b'0' as u32),
        0x10 => Some(b'q' as u32),
        0x11 => Some(b'w' as u32),
        0x12 => Some(b'e' as u32),
        0x13 => Some(b'r' as u32),
        0x14 => Some(b't' as u32),
        0x15 => Some(b'y' as u32),
        0x16 => Some(b'u' as u32),
        0x17 => Some(b'i' as u32),
        0x18 => Some(b'o' as u32),
        0x19 => Some(b'p' as u32),
        0x1E => Some(b'a' as u32),
        0x1F => Some(b's' as u32),
        0x20 => Some(b'd' as u32),
        0x21 => Some(b'f' as u32),
        0x22 => Some(b'g' as u32),
        0x23 => Some(b'h' as u32),
        0x24 => Some(b'j' as u32),
        0x25 => Some(b'k' as u32),
        0x26 => Some(b'l' as u32),
        0x2C => Some(b'z' as u32),
        0x2D => Some(b'x' as u32),
        0x2E => Some(b'c' as u32),
        0x2F => Some(b'v' as u32),
        0x30 => Some(b'b' as u32),
        0x31 => Some(b'n' as u32),
        0x32 => Some(b'm' as u32),
        0x39 => Some(b' ' as u32),
        _ => None,
    }
}

// ================= VNC Rectangle Decoders (Raw, Hextile, Zlib, ZRLE) =================

#[inline]
fn set_opaque_alpha(pixel_data: &mut [u8]) {
    for px in pixel_data.chunks_exact_mut(4) {
        px[3] = 0xFF;
    }
}

#[inline]
fn fill_subrect(
    buf: &mut [u8],
    rect_w: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: [u8; 4],
) {
    let rect_h = buf.len() / (rect_w * 4);
    let max_y = (y + h).min(rect_h);
    let max_x = (x + w).min(rect_w);
    for row in y..max_y {
        let row_offset = row * rect_w * 4;
        for col in x..max_x {
            let idx = row_offset + col * 4;
            buf[idx..idx + 4].copy_from_slice(&color);
        }
    }
}

fn decode_hextile_rect<R: Read>(
    stream: &mut R,
    rw: u16,
    rh: u16,
) -> std::io::Result<Vec<u8>> {
    let w = rw as usize;
    let h = rh as usize;
    let mut out = vec![0u8; w * h * 4];
    let mut bg = [0u8, 0u8, 0u8, 0xFF];
    let mut fg = [0u8, 0u8, 0u8, 0xFF];

    for ty in (0..h).step_by(16) {
        let th = (h - ty).min(16);
        for tx in (0..w).step_by(16) {
            let tw = (w - tx).min(16);
            let mut subenc_buf = [0u8; 1];
            stream.read_exact(&mut subenc_buf)?;
            let subenc = subenc_buf[0];

            if (subenc & 1) != 0 {
                // Raw tile
                let mut raw_tile = vec![0u8; tw * th * 4];
                stream.read_exact(&mut raw_tile)?;
                for row in 0..th {
                    let src_off = row * tw * 4;
                    let dst_off = ((ty + row) * w + tx) * 4;
                    out[dst_off..dst_off + tw * 4]
                        .copy_from_slice(&raw_tile[src_off..src_off + tw * 4]);
                    for col in 0..tw {
                        out[dst_off + col * 4 + 3] = 0xFF;
                    }
                }
                continue;
            }

            if (subenc & 2) != 0 {
                // BackgroundSpecified
                stream.read_exact(&mut bg)?;
                bg[3] = 0xFF;
            }
            fill_subrect(&mut out, w, tx, ty, tw, th, bg);

            if (subenc & 4) != 0 {
                // ForegroundSpecified
                stream.read_exact(&mut fg)?;
                fg[3] = 0xFF;
            }

            if (subenc & 8) != 0 {
                // AnySubrects
                let mut n_sub_buf = [0u8; 1];
                stream.read_exact(&mut n_sub_buf)?;
                let n_sub = n_sub_buf[0] as usize;
                let colored = (subenc & 16) != 0;

                for _ in 0..n_sub {
                    let sub_color = if colored {
                        let mut c = [0u8; 4];
                        stream.read_exact(&mut c)?;
                        c[3] = 0xFF;
                        c
                    } else {
                        fg
                    };
                    let mut coords = [0u8; 2];
                    stream.read_exact(&mut coords)?;
                    let sx = ((coords[0] >> 4) & 0x0f) as usize;
                    let sy = (coords[0] & 0x0f) as usize;
                    let sw = (((coords[1] >> 4) & 0x0f) as usize) + 1;
                    let sh = ((coords[1] & 0x0f) as usize) + 1;
                    fill_subrect(&mut out, w, tx + sx, ty + sy, sw, sh, sub_color);
                }
            }
        }
    }

    Ok(out)
}

fn inflate_stream(decompressor: &mut Decompress, compressed: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(compressed.len() * 4 + 4096);
    let mut in_offset = 0;
    let mut chunk = [0u8; 32768];

    while in_offset < compressed.len() {
        let before_in = decompressor.total_in();
        let before_out = decompressor.total_out();
        let status = decompressor
            .decompress(&compressed[in_offset..], &mut chunk, FlushDecompress::Sync)
            .map_err(|e| format!("VNC zlib decompression error: {}", e))?;
        let consumed = (decompressor.total_in() - before_in) as usize;
        let produced = (decompressor.total_out() - before_out) as usize;
        in_offset += consumed;
        if produced > 0 {
            out.extend_from_slice(&chunk[..produced]);
        }
        if status == flate2::Status::StreamEnd || (consumed == 0 && produced == 0) {
            break;
        }
    }

    // Drain any remaining output bytes
    loop {
        let before_out = decompressor.total_out();
        let _ = decompressor.decompress(&[], &mut chunk, FlushDecompress::None);
        let produced = (decompressor.total_out() - before_out) as usize;
        if produced > 0 {
            out.extend_from_slice(&chunk[..produced]);
        } else {
            break;
        }
    }

    Ok(out)
}

fn decode_zrle_rect(data: &[u8], rw: u16, rh: u16) -> Result<Vec<u8>, String> {
    let w = rw as usize;
    let h = rh as usize;
    let mut out = vec![0u8; w * h * 4];
    let mut pos = 0usize;

    let read_cpixel = |pos: &mut usize| -> Result<[u8; 4], String> {
        if *pos + 3 > data.len() {
            return Err("ZRLE truncated CPIXEL".into());
        }
        let c = [data[*pos], data[*pos + 1], data[*pos + 2], 0xFF];
        *pos += 3;
        Ok(c)
    };

    for ty in (0..h).step_by(64) {
        let th = (h - ty).min(64);
        for tx in (0..w).step_by(64) {
            let tw = (w - tx).min(64);
            if pos >= data.len() {
                return Err("ZRLE truncated tile header".into());
            }
            let subenc = data[pos];
            pos += 1;

            if subenc == 0 {
                // Raw CPIXELs
                for row in 0..th {
                    let dst_row = ((ty + row) * w + tx) * 4;
                    for col in 0..tw {
                        let c = read_cpixel(&mut pos)?;
                        let idx = dst_row + col * 4;
                        out[idx..idx + 4].copy_from_slice(&c);
                    }
                }
            } else if subenc == 1 {
                // Solid tile
                let c = read_cpixel(&mut pos)?;
                fill_subrect(&mut out, w, tx, ty, tw, th, c);
            } else if (2..=16).contains(&subenc) {
                // Packed palette
                let pal_size = subenc as usize;
                let mut palette = [[0u8; 4]; 16];
                for p in palette.iter_mut().take(pal_size) {
                    *p = read_cpixel(&mut pos)?;
                }
                let bpp = if pal_size == 2 {
                    1
                } else if pal_size <= 4 {
                    2
                } else {
                    4
                };
                let mask = (1u8 << bpp) - 1;

                for row in 0..th {
                    let dst_row = ((ty + row) * w + tx) * 4;
                    let mut shift = 0i32;
                    let mut cur_byte = 0u8;
                    for col in 0..tw {
                        if shift == 0 {
                            if pos >= data.len() {
                                return Err("ZRLE truncated packed palette indices".into());
                            }
                            cur_byte = data[pos];
                            pos += 1;
                            shift = 8;
                        }
                        shift -= bpp as i32;
                        let pal_idx = ((cur_byte >> shift) & mask) as usize;
                        let c = palette[pal_idx.min(pal_size - 1)];
                        let idx = dst_row + col * 4;
                        out[idx..idx + 4].copy_from_slice(&c);
                    }
                }
            } else if subenc == 128 {
                // Plain RLE
                let total_pixels = tw * th;
                let mut pix_idx = 0usize;
                while pix_idx < total_pixels {
                    let c = read_cpixel(&mut pos)?;
                    let mut run_len = 1usize;
                    loop {
                        if pos >= data.len() {
                            return Err("ZRLE truncated plain RLE length".into());
                        }
                        let b = data[pos];
                        pos += 1;
                        run_len += b as usize;
                        if b != 255 {
                            break;
                        }
                    }
                    for _ in 0..run_len {
                        if pix_idx >= total_pixels {
                            break;
                        }
                        let row = pix_idx / tw;
                        let col = pix_idx % tw;
                        let idx = ((ty + row) * w + (tx + col)) * 4;
                        out[idx..idx + 4].copy_from_slice(&c);
                        pix_idx += 1;
                    }
                }
            } else if subenc >= 130 {
                // Palette RLE
                let pal_size = (subenc - 128) as usize;
                let mut palette = [[0u8; 4]; 128];
                for p in palette.iter_mut().take(pal_size) {
                    *p = read_cpixel(&mut pos)?;
                }
                let total_pixels = tw * th;
                let mut pix_idx = 0usize;
                while pix_idx < total_pixels {
                    if pos >= data.len() {
                        return Err("ZRLE truncated palette RLE index".into());
                    }
                    let index_byte = data[pos];
                    pos += 1;
                    let pal_idx = (index_byte & 127) as usize;
                    let c = palette[pal_idx.min(pal_size.saturating_sub(1))];
                    let mut run_len = 1usize;
                    if (index_byte & 128) != 0 {
                        loop {
                            if pos >= data.len() {
                                return Err("ZRLE truncated palette RLE length".into());
                            }
                            let b = data[pos];
                            pos += 1;
                            run_len += b as usize;
                            if b != 255 {
                                break;
                            }
                        }
                    }
                    for _ in 0..run_len {
                        if pix_idx >= total_pixels {
                            break;
                        }
                        let row = pix_idx / tw;
                        let col = pix_idx % tw;
                        let idx = ((ty + row) * w + (tx + col)) * 4;
                        out[idx..idx + 4].copy_from_slice(&c);
                        pix_idx += 1;
                    }
                }
            } else {
                return Err(format!("Unknown ZRLE subencoding {}", subenc));
            }
        }
    }

    Ok(out)
}

// ================= Main VNC WebSocket Gateway Session =================

pub async fn handle_vnc_session(mut socket: WebSocket, params: VncConnectionParams) {
    let host_label = format!("{}:{}", params.host, params.port);
    let (mut stream, mut info) =
        match tokio::task::spawn_blocking(move || connect_and_handshake_vnc(&params)).await {
            Ok(Ok(res)) => res,
            Ok(Err(e)) => {
                warn!("VNC connection to {} failed: {}", host_label, e);
                let _ = socket
                    .send(Message::Text(
                        serde_json::json!({
                            "type": "error",
                            "message": format!("VNC Connection failed: {}", e)
                        })
                        .to_string(),
                    ))
                    .await;
                return;
            }
            Err(e) => {
                error!("Internal VNC task error for {}: {}", host_label, e);
                let _ = socket
                    .send(Message::Text(
                        serde_json::json!({
                            "type": "error",
                            "message": format!("Internal VNC task error: {}", e)
                        })
                        .to_string(),
                    ))
                    .await;
                return;
            }
        };

    // Send Init message to frontend
    let _ = socket
        .send(Message::Text(
            serde_json::json!({
                "type": "init",
                "protocol": "vnc",
                "width": info.width,
                "height": info.height,
                "name": info.name
            })
            .to_string(),
        ))
        .await;

    let (ws_sender_tx, mut ws_sender_rx) = mpsc::channel::<Message>(512);
    let (vnc_write_tx, mut vnc_write_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let is_running = Arc::new(AtomicBool::new(true));

    // Task 1: Single-owner VNC RFB stream loop (handles both reads & writes cleanly for TCP and TLS)
    let is_running_clone = is_running.clone();
    let vnc_io_handle = tokio::task::spawn_blocking(move || {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
        let mut zlib_decompressor = Decompress::new(true);
        let mut zrle_decompressor = Decompress::new(true);
        let mut ext_desktop_screen: Option<(u32, u32)> = None; // (screen_id, flags)

        let mut msg_type_buf = [0u8; 1];
        while is_running_clone.load(Ordering::Relaxed) {
            // 1. Drain any pending client-to-server packets (mouse, keys, clipboard, resize)
            let mut wrote_any = false;
            while let Ok(pkt) = vnc_write_rx.try_recv() {
                if pkt.len() >= 7 && pkt[0] == 251 && pkt[1] == 0xFF {
                    // Internal marker for dynamic resize request: [251, 0xFF, w_hi, w_lo, h_hi, h_lo]
                    if let Some((screen_id, flags)) = ext_desktop_screen {
                        let req_w = u16::from_be_bytes([pkt[2], pkt[3]]);
                        let req_h = u16::from_be_bytes([pkt[4], pkt[5]]);
                        let mut sds = [0u8; 24];
                        sds[0] = 251; // SetDesktopSize
                        sds[2..4].copy_from_slice(&req_w.to_be_bytes());
                        sds[4..6].copy_from_slice(&req_h.to_be_bytes());
                        sds[6] = 1; // 1 screen
                        sds[8..12].copy_from_slice(&screen_id.to_be_bytes());
                        sds[16..18].copy_from_slice(&req_w.to_be_bytes());
                        sds[18..20].copy_from_slice(&req_h.to_be_bytes());
                        sds[20..24].copy_from_slice(&flags.to_be_bytes());
                        if stream.write_all(&sds).is_err() {
                            is_running_clone.store(false, Ordering::Relaxed);
                            break;
                        }
                        wrote_any = true;
                    }
                    continue;
                }
                if stream.write_all(&pkt).is_err() {
                    is_running_clone.store(false, Ordering::Relaxed);
                    break;
                }
                wrote_any = true;
            }
            if wrote_any && stream.flush().is_err() {
                break;
            }
            if !is_running_clone.load(Ordering::Relaxed) {
                break;
            }

            // 2. Poll up to 6ms for incoming VNC server data
            match stream.has_pending_or_readable(6) {
                Ok(false) => continue,
                Ok(true) => {}
                Err(_) => break,
            }

            if stream.read_exact(&mut msg_type_buf).is_err() {
                break;
            }

            match msg_type_buf[0] {
                0 => {
                    // FramebufferUpdate
                    let mut hdr = [0u8; 3]; // 1 padding, 2 num_rectangles
                    if stream.read_exact(&mut hdr).is_err() {
                        break;
                    }
                    let num_rects = u16::from_be_bytes([hdr[1], hdr[2]]);

                    for _ in 0..num_rects {
                        // Also flush any mouse/key events between rectangles for low input latency
                        let mut flushed_input = false;
                        while let Ok(pkt) = vnc_write_rx.try_recv() {
                            if pkt.len() >= 7 && pkt[0] == 251 && pkt[1] == 0xFF {
                                continue;
                            }
                            let _ = stream.write_all(&pkt);
                            flushed_input = true;
                        }
                        if flushed_input {
                            let _ = stream.flush();
                        }

                        let mut rect_hdr = [0u8; 12];
                        if stream.read_exact(&mut rect_hdr).is_err() {
                            is_running_clone.store(false, Ordering::Relaxed);
                            break;
                        }
                        let rx = u16::from_be_bytes([rect_hdr[0], rect_hdr[1]]);
                        let ry = u16::from_be_bytes([rect_hdr[2], rect_hdr[3]]);
                        let rw = u16::from_be_bytes([rect_hdr[4], rect_hdr[5]]);
                        let rh = u16::from_be_bytes([rect_hdr[6], rect_hdr[7]]);
                        let encoding = i32::from_be_bytes([
                            rect_hdr[8],
                            rect_hdr[9],
                            rect_hdr[10],
                            rect_hdr[11],
                        ]);

                        match encoding {
                            0 => {
                                // Raw 32-bit RGBA pixel bytes
                                let pixel_bytes_len = (rw as usize) * (rh as usize) * 4;
                                let mut pixel_data = vec![0u8; pixel_bytes_len];
                                if stream.read_exact(&mut pixel_data).is_err() {
                                    is_running_clone.store(false, Ordering::Relaxed);
                                    break;
                                }
                                set_opaque_alpha(&mut pixel_data);

                                let mut packet = Vec::with_capacity(9 + pixel_bytes_len);
                                packet.push(0x01); // Frame Tile
                                packet.extend_from_slice(&rx.to_be_bytes());
                                packet.extend_from_slice(&ry.to_be_bytes());
                                packet.extend_from_slice(&rw.to_be_bytes());
                                packet.extend_from_slice(&rh.to_be_bytes());
                                packet.extend_from_slice(&pixel_data);

                                if ws_sender_tx.blocking_send(Message::Binary(packet)).is_err() {
                                    is_running_clone.store(false, Ordering::Relaxed);
                                    break;
                                }
                            }
                            1 => {
                                // CopyRect
                                let mut copy_src = [0u8; 4];
                                if stream.read_exact(&mut copy_src).is_err() {
                                    is_running_clone.store(false, Ordering::Relaxed);
                                    break;
                                }
                                let src_x = u16::from_be_bytes([copy_src[0], copy_src[1]]);
                                let src_y = u16::from_be_bytes([copy_src[2], copy_src[3]]);

                                let mut packet = Vec::with_capacity(13);
                                packet.push(0x02); // CopyRect command
                                packet.extend_from_slice(&rx.to_be_bytes());
                                packet.extend_from_slice(&ry.to_be_bytes());
                                packet.extend_from_slice(&rw.to_be_bytes());
                                packet.extend_from_slice(&rh.to_be_bytes());
                                packet.extend_from_slice(&src_x.to_be_bytes());
                                packet.extend_from_slice(&src_y.to_be_bytes());

                                if ws_sender_tx.blocking_send(Message::Binary(packet)).is_err() {
                                    is_running_clone.store(false, Ordering::Relaxed);
                                    break;
                                }
                            }
                            5 => {
                                // Hextile (RFC 6143 Section 7.7.4)
                                match decode_hextile_rect(&mut stream, rw, rh) {
                                    Ok(pixel_data) => {
                                        let mut packet = Vec::with_capacity(9 + pixel_data.len());
                                        packet.push(0x01);
                                        packet.extend_from_slice(&rx.to_be_bytes());
                                        packet.extend_from_slice(&ry.to_be_bytes());
                                        packet.extend_from_slice(&rw.to_be_bytes());
                                        packet.extend_from_slice(&rh.to_be_bytes());
                                        packet.extend_from_slice(&pixel_data);

                                        if ws_sender_tx
                                            .blocking_send(Message::Binary(packet))
                                            .is_err()
                                        {
                                            is_running_clone.store(false, Ordering::Relaxed);
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        warn!("VNC Hextile decode error: {}", e);
                                        is_running_clone.store(false, Ordering::Relaxed);
                                        break;
                                    }
                                }
                            }
                            6 => {
                                // Zlib compressed Raw pixels
                                let mut len_buf = [0u8; 4];
                                if stream.read_exact(&mut len_buf).is_err() {
                                    is_running_clone.store(false, Ordering::Relaxed);
                                    break;
                                }
                                let zlen = u32::from_be_bytes(len_buf) as usize;
                                let mut zbuf = vec![0u8; zlen];
                                if stream.read_exact(&mut zbuf).is_err() {
                                    is_running_clone.store(false, Ordering::Relaxed);
                                    break;
                                }
                                match inflate_stream(&mut zlib_decompressor, &zbuf) {
                                    Ok(mut pixel_data) => {
                                        let expected = (rw as usize) * (rh as usize) * 4;
                                        pixel_data.resize(expected, 0xFF);
                                        set_opaque_alpha(&mut pixel_data);

                                        let mut packet = Vec::with_capacity(9 + pixel_data.len());
                                        packet.push(0x01);
                                        packet.extend_from_slice(&rx.to_be_bytes());
                                        packet.extend_from_slice(&ry.to_be_bytes());
                                        packet.extend_from_slice(&rw.to_be_bytes());
                                        packet.extend_from_slice(&rh.to_be_bytes());
                                        packet.extend_from_slice(&pixel_data);

                                        if ws_sender_tx
                                            .blocking_send(Message::Binary(packet))
                                            .is_err()
                                        {
                                            is_running_clone.store(false, Ordering::Relaxed);
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        warn!("VNC Zlib decode error: {}", e);
                                        is_running_clone.store(false, Ordering::Relaxed);
                                        break;
                                    }
                                }
                            }
                            16 => {
                                // ZRLE (RFC 6143 Section 7.7.6)
                                let mut len_buf = [0u8; 4];
                                if stream.read_exact(&mut len_buf).is_err() {
                                    is_running_clone.store(false, Ordering::Relaxed);
                                    break;
                                }
                                let zlen = u32::from_be_bytes(len_buf) as usize;
                                let mut zbuf = vec![0u8; zlen];
                                if stream.read_exact(&mut zbuf).is_err() {
                                    is_running_clone.store(false, Ordering::Relaxed);
                                    break;
                                }
                                match inflate_stream(&mut zrle_decompressor, &zbuf)
                                    .and_then(|inflated| decode_zrle_rect(&inflated, rw, rh))
                                {
                                    Ok(pixel_data) => {
                                        let mut packet = Vec::with_capacity(9 + pixel_data.len());
                                        packet.push(0x01);
                                        packet.extend_from_slice(&rx.to_be_bytes());
                                        packet.extend_from_slice(&ry.to_be_bytes());
                                        packet.extend_from_slice(&rw.to_be_bytes());
                                        packet.extend_from_slice(&rh.to_be_bytes());
                                        packet.extend_from_slice(&pixel_data);

                                        if ws_sender_tx
                                            .blocking_send(Message::Binary(packet))
                                            .is_err()
                                        {
                                            is_running_clone.store(false, Ordering::Relaxed);
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        warn!("VNC ZRLE decode error: {}", e);
                                        is_running_clone.store(false, Ordering::Relaxed);
                                        break;
                                    }
                                }
                            }
                            -223 => {
                                // DesktopSize pseudo-encoding
                                if rw > 0 && rh > 0 {
                                    info.width = rw;
                                    info.height = rh;
                                    let _ = ws_sender_tx.blocking_send(Message::Text(
                                        serde_json::json!({
                                            "type": "init",
                                            "protocol": "vnc",
                                            "width": info.width,
                                            "height": info.height,
                                            "name": info.name
                                        })
                                        .to_string(),
                                    ));
                                }
                            }
                            -308 => {
                                // ExtendedDesktopSize pseudo-encoding
                                let mut eds_hdr = [0u8; 4]; // num_screens: u8 + 3 padding
                                if stream.read_exact(&mut eds_hdr).is_err() {
                                    is_running_clone.store(false, Ordering::Relaxed);
                                    break;
                                }
                                let num_screens = eds_hdr[0] as usize;
                                for s_idx in 0..num_screens {
                                    let mut scr = [0u8; 16];
                                    if stream.read_exact(&mut scr).is_err() {
                                        is_running_clone.store(false, Ordering::Relaxed);
                                        break;
                                    }
                                    if s_idx == 0 {
                                        let sid = u32::from_be_bytes([scr[0], scr[1], scr[2], scr[3]]);
                                        let sflags = u32::from_be_bytes([scr[12], scr[13], scr[14], scr[15]]);
                                        ext_desktop_screen = Some((sid, sflags));
                                    }
                                }
                                if rw > 0 && rh > 0 && (rw != info.width || rh != info.height) {
                                    info.width = rw;
                                    info.height = rh;
                                    let _ = ws_sender_tx.blocking_send(Message::Text(
                                        serde_json::json!({
                                            "type": "init",
                                            "protocol": "vnc",
                                            "width": info.width,
                                            "height": info.height,
                                            "name": info.name
                                        })
                                        .to_string(),
                                    ));
                                }
                            }
                            -239 => {
                                // Rich Cursor pseudo-encoding: consume cursor pixels + bitmask
                                let pix_len = (rw as usize) * (rh as usize) * 4;
                                let mask_len = ((rw as usize + 7) / 8) * (rh as usize);
                                let mut discard = vec![0u8; pix_len + mask_len];
                                if stream.read_exact(&mut discard).is_err() {
                                    is_running_clone.store(false, Ordering::Relaxed);
                                    break;
                                }
                            }
                            -224 => {
                                // LastRect pseudo-encoding: end of rectangles list
                                break;
                            }
                            other => {
                                warn!("VNC Gateway: Unexpected rectangle encoding: {}", other);
                                is_running_clone.store(false, Ordering::Relaxed);
                                break;
                            }
                        }
                    }

                    // Request next incremental frame update
                    let req = build_fbur_packet(1, 0, 0, info.width, info.height);
                    let _ = stream.write_all(&req);
                    let _ = stream.flush();
                }
                1 => {
                    // SetColourMapEntries: [padding: u8, first_color: u16, num_colors: u16, rgb: num_colors * 6]
                    let mut hdr = [0u8; 5];
                    if stream.read_exact(&mut hdr).is_err() {
                        break;
                    }
                    let num_colors = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
                    let mut colors = vec![0u8; num_colors * 6];
                    if stream.read_exact(&mut colors).is_err() {
                        break;
                    }
                }
                2 => {
                    // Bell (0 bytes payload)
                }
                3 => {
                    // ServerCutText (Clipboard change on remote server)
                    let mut pad = [0u8; 3];
                    if stream.read_exact(&mut pad).is_err() {
                        break;
                    }
                    let mut len_buf = [0u8; 4];
                    if stream.read_exact(&mut len_buf).is_ok() {
                        let text_len = u32::from_be_bytes(len_buf) as usize;
                        if text_len <= 10 * 1024 * 1024 {
                            let mut text_buf = vec![0u8; text_len];
                            if stream.read_exact(&mut text_buf).is_ok() {
                                let text = String::from_utf8_lossy(&text_buf).to_string();
                                let msg = serde_json::json!({
                                    "type": "clipboard_sync",
                                    "text": text
                                });
                                let _ = ws_sender_tx.blocking_send(Message::Text(msg.to_string()));
                            }
                        }
                    }
                }
                other => {
                    warn!("VNC Gateway: Unknown server message type: {}", other);
                    break;
                }
            }
        }
        is_running_clone.store(false, Ordering::Relaxed);
    });

    // Task 2: Dispatch WebSocket outgoing
    let (mut ws_sink, mut ws_stream) = socket.split();
    let is_running_ws = is_running.clone();
    let mut ws_send_handle = tokio::spawn(async move {
        while let Some(msg) = ws_sender_rx.recv().await {
            if ws_sink.send(msg).await.is_err() {
                break;
            }
        }
        let _ = ws_sink.close().await;
        is_running_ws.store(false, Ordering::Relaxed);
    });

    // Task 3: Handle incoming client inputs from browser
    loop {
        tokio::select! {
            _ = &mut ws_send_handle => {
                break;
            }
            msg_opt = ws_stream.next() => {
                match msg_opt {
                    Some(Ok(msg)) => {
                        match msg {
                            Message::Binary(bin) => {
                                if bin.is_empty() {
                                    continue;
                                }
                                match bin[0] {
                                    0x02 => {
                                        // Pointer Event: [0x02, mask: u8, x: u16, y: u16]
                                        if bin.len() >= 6 {
                                            let mask = bin[1];
                                            let x = u16::from_be_bytes([bin[2], bin[3]]);
                                            let y = u16::from_be_bytes([bin[4], bin[5]]);

                                            let mut pe = Vec::with_capacity(6);
                                            pe.push(5); // PointerEvent
                                            pe.push(mask);
                                            pe.extend_from_slice(&x.to_be_bytes());
                                            pe.extend_from_slice(&y.to_be_bytes());
                                            let _ = vnc_write_tx.send(pe);
                                        }
                                    }
                                    0x04 => {
                                        // Key Event from browser: [0x04, down: u8, scancode: u8, extended: u8, unicode: u16]
                                        if bin.len() >= 6 {
                                            let down = bin[1];
                                            let scancode = bin[2];
                                            let extended = bin[3];
                                            let unicode = u16::from_be_bytes([bin[4], bin[5]]);

                                            if let Some(keysym) = map_browser_key_to_keysym(scancode, extended, unicode) {
                                                let ke = build_key_event(down, keysym);
                                                let _ = vnc_write_tx.send(ke.to_vec());
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Message::Text(txt) => {
                                if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                                    if let Some(msg_type) = val.get("type").and_then(|t| t.as_str()) {
                                        match msg_type {
                                            "clipboard_push" => {
                                                if let Some(content) = val.get("text").and_then(|t| t.as_str()) {
                                                    let content_bytes = content.as_bytes();
                                                    let mut cut_msg = Vec::with_capacity(8 + content_bytes.len());
                                                    cut_msg.push(6); // ClientCutText
                                                    cut_msg.extend_from_slice(&[0, 0, 0]); // 3 bytes padding
                                                    cut_msg.extend_from_slice(&(content_bytes.len() as u32).to_be_bytes());
                                                    cut_msg.extend_from_slice(content_bytes);
                                                    let _ = vnc_write_tx.send(cut_msg);

                                                    // Synthesize Ctrl+v after updating remote clipboard so paste happens immediately
                                                    let tx_clone = vnc_write_tx.clone();
                                                    tokio::spawn(async move {
                                                        tokio::time::sleep(Duration::from_millis(30)).await;
                                                        let mut seq = Vec::with_capacity(32);
                                                        seq.extend_from_slice(&build_key_event(1, 0xffe3)); // Control_L down
                                                        seq.extend_from_slice(&build_key_event(1, 0x0076)); // 'v' down
                                                        seq.extend_from_slice(&build_key_event(0, 0x0076)); // 'v' up
                                                        seq.extend_from_slice(&build_key_event(0, 0xffe3)); // Control_L up
                                                        let _ = tx_clone.send(seq);
                                                    });
                                                }
                                            }
                                            "resize" => {
                                                if let (Some(w), Some(h)) = (
                                                    val.get("width").and_then(|v| v.as_u64()),
                                                    val.get("height").and_then(|v| v.as_u64()),
                                                ) {
                                                    let rw = (w as u16).clamp(640, 3840);
                                                    let rh = (h as u16).clamp(480, 2160);
                                                    let mut pkt = vec![251, 0xFF];
                                                    pkt.extend_from_slice(&rw.to_be_bytes());
                                                    pkt.extend_from_slice(&rh.to_be_bytes());
                                                    let _ = vnc_write_tx.send(pkt);
                                                }
                                            }
                                            "refresh" => {
                                                let fbur = build_fbur_packet(0, 0, 0, info.width, info.height);
                                                let _ = vnc_write_tx.send(fbur.to_vec());
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            Message::Close(_) => break,
                            _ => {}
                        }
                    }
                    _ => break,
                }
            }
        }
    }

    is_running.store(false, Ordering::Relaxed);
    ws_send_handle.abort();
    let _ = vnc_io_handle.await;
    info!("VNC session closed");
}
