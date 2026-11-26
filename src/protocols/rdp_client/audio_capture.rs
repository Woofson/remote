//! Remote addition (#37): tell Windows that this client can capture audio.
//!
//! Windows only opens the microphone channel (AUDIO_INPUT) for clients that set the
//! INFO_AUDIOCAPTURE flag in the Client Info PDU (MS-RDPBCGR 2.2.1.11.1.1). The connector
//! builds that PDU internally and has no option for the flag, so this wrapper around the
//! encrypted connection sets the bit on the one outgoing Client Info PDU. Only that bit
//! changes; no length changes, and it happens at most once per connection.

use core::pin::Pin;
use core::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// INFO_AUDIOCAPTURE in the Client Info flags.
const INFO_AUDIOCAPTURE: u32 = 0x0020_0000;
/// SEC_INFO_PKT in the basic security header: marks the Client Info PDU.
const SEC_INFO_PKT: u16 = 0x0040;
/// SEC_ENCRYPT: standard RDP encryption (never used with TLS); then the flags are not readable.
const SEC_ENCRYPT: u16 = 0x0008;

pub struct AudioCaptureFlag<S> {
    inner: S,
    /// Still looking for the Client Info PDU.
    armed: bool,
    /// The patched frame, while it is being written.
    patched: Option<(Vec<u8>, usize)>,
}

impl<S> AudioCaptureFlag<S> {
    pub fn new(inner: S, enabled: bool) -> Self {
        Self { inner, armed: enabled, patched: None }
    }
}

/// If `frame` is one whole Client Info PDU (TPKT + X.224 data + MCS Send Data Request +
/// basic security header with SEC_INFO_PKT), the position of its flags field.
fn client_info_flags_at(frame: &[u8]) -> Option<usize> {
    if frame.len() < 30 || frame[0] != 3 {
        return None;
    }
    let tpkt_len = u16::from_be_bytes([frame[2], frame[3]]) as usize;
    // X.224 data TPDU: length indicator 2, code 0xF0, end of transmission.
    if tpkt_len != frame.len() || frame[4] != 2 || frame[5] != 0xF0 || frame[6] != 0x80 {
        return None;
    }
    // MCS Send Data Request (choice 25), then initiator, channel, priority/segmentation.
    if frame[7] >> 2 != 25 {
        return None;
    }
    let (len, data) = if frame[13] & 0x80 != 0 {
        ((((frame[13] & 0x7F) as usize) << 8) | frame[14] as usize, 15)
    } else {
        (frame[13] as usize, 14)
    };
    if data + len != frame.len() || len < 12 {
        return None;
    }
    let sec_flags = u16::from_le_bytes([frame[data], frame[data + 1]]);
    if sec_flags & SEC_INFO_PKT == 0 || sec_flags & SEC_ENCRYPT != 0 {
        return None;
    }
    // Security header (4 bytes), then CodePage (4 bytes), then the flags.
    Some(data + 8)
}

impl<S: AsyncWrite + Unpin> AsyncWrite for AudioCaptureFlag<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let this = &mut *self;
        if this.patched.is_none() && this.armed {
            if let Some(at) = client_info_flags_at(buf) {
                let mut copy = buf.to_vec();
                let flags = u32::from_le_bytes([copy[at], copy[at + 1], copy[at + 2], copy[at + 3]]) | INFO_AUDIOCAPTURE;
                copy[at..at + 4].copy_from_slice(&flags.to_le_bytes());
                this.armed = false;
                this.patched = Some((copy, 0));
            }
        }
        if let Some((copy, done)) = this.patched.as_mut() {
            // The caller re-offers the same bytes until they are all written.
            let n = match Pin::new(&mut this.inner).poll_write(cx, &copy[*done..]) {
                Poll::Ready(Ok(n)) => n,
                other => return other,
            };
            *done += n;
            if *done >= copy.len() {
                this.patched = None;
            }
            return Poll::Ready(Ok(n));
        }
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for AudioCaptureFlag<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_core::{ReadCursor, WriteBuf};
    use ironrdp_pdu::rdp::client_info::{
        AddressFamily, ClientInfo, ClientInfoFlags, CompressionType, Credentials, ExtendedClientInfo, ExtendedClientOptionalInfo,
    };
    use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
    use ironrdp_pdu::rdp::ClientInfoPdu;
    use tokio::io::AsyncWriteExt;

    /// A real Client Info PDU frame, encoded by the RDP library the way the connector sends it.
    fn client_info_frame(username: &str) -> Vec<u8> {
        let pdu = ClientInfoPdu {
            security_header: BasicSecurityHeader { flags: BasicSecurityHeaderFlags::INFO_PKT },
            client_info: ClientInfo {
                credentials: Credentials { username: username.into(), password: "secret".into(), domain: Some("TEST".into()) },
                code_page: 0,
                flags: ClientInfoFlags::MOUSE | ClientInfoFlags::UNICODE | ClientInfoFlags::LOGON_NOTIFY,
                compression_type: CompressionType::K8,
                alternate_shell: String::new(),
                work_dir: String::new(),
                extra_info: ExtendedClientInfo {
                    address_family: AddressFamily::INET,
                    address: "192.0.2.1".into(),
                    dir: "C:\\".into(),
                    optional_data: ExtendedClientOptionalInfo::default(),
                },
            },
        };
        let mut buf = WriteBuf::new();
        ironrdp_connector::encode_send_data_request(1007, 1003, &pdu, &mut buf).unwrap();
        buf.into_inner()
    }

    fn decode_flags(frame: &[u8]) -> ClientInfoFlags {
        let at = client_info_flags_at(frame).expect("a Client Info frame");
        let mut cur = ReadCursor::new(&frame[at - 8..]);
        let pdu: ClientInfoPdu = ironrdp_core::decode_cursor(&mut cur).unwrap();
        pdu.client_info.flags
    }

    async fn send_through(enabled: bool, frames: &[Vec<u8>]) -> Vec<u8> {
        let mut w = AudioCaptureFlag::new(Vec::new(), enabled);
        for f in frames {
            w.write_all(f).await.unwrap();
        }
        w.inner
    }

    #[tokio::test]
    async fn the_flag_is_set_on_the_client_info_pdu_only() {
        let info = client_info_frame("alice");
        // A long user name makes the MCS length take two bytes.
        let long = client_info_frame(&"x".repeat(200));
        let other = vec![3, 0, 0, 12, 2, 0xF0, 0x80, 0x64, 0, 1, 3, 0xEB];
        assert!(!decode_flags(&info).contains(ClientInfoFlags::AUDIO_CAPTURE));

        let out = send_through(true, &[other.clone(), info.clone()]).await;
        assert_eq!(&out[..other.len()], &other[..], "other frames pass unchanged");
        let sent = &out[other.len()..];
        assert_eq!(sent.len(), info.len(), "no length change");
        let flags = decode_flags(sent);
        assert!(flags.contains(ClientInfoFlags::AUDIO_CAPTURE));
        assert!(flags.contains(ClientInfoFlags::MOUSE | ClientInfoFlags::UNICODE | ClientInfoFlags::LOGON_NOTIFY), "other flags kept");
        let changed: Vec<usize> = (0..info.len()).filter(|&i| info[i] != sent[i]).collect();
        assert_eq!(changed.len(), 1, "exactly one byte differs");

        let out = send_through(true, std::slice::from_ref(&long)).await;
        assert!(decode_flags(&out).contains(ClientInfoFlags::AUDIO_CAPTURE));

        // Only once per connection, and never when switched off.
        let out = send_through(true, &[info.clone(), info.clone()]).await;
        assert!(!decode_flags(&out[info.len()..]).contains(ClientInfoFlags::AUDIO_CAPTURE));
        let out = send_through(false, std::slice::from_ref(&info)).await;
        assert_eq!(out, info);
    }
}
