//! Microphone into RDP sessions (#37): the client side of the audio input channel
//! (dynamic virtual channel "AUDIO_INPUT", MS-RDPEAI). Windows opens the channel when a
//! program starts recording; Remote then asks the browser for sound and forwards it as
//! 16-bit PCM in the format Windows picked.
//!
//! Messages (all little-endian, the first byte is the message id):
//! 0x01 version, 0x02 formats, 0x03 open, 0x04 open reply, 0x05 data incoming,
//! 0x06 data, 0x07 format change.

use ironrdp_core::{ensure_size, Encode, EncodeResult, WriteCursor};
use ironrdp_dvc::{DvcEncode, DvcMessage, DvcProcessor};
use ironrdp_pdu::{pdu_other_err, PduResult};
use parking_lot::Mutex;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub const CHANNEL_NAME: &str = "AUDIO_INPUT";

const MSG_VERSION: u8 = 0x01;
const MSG_FORMATS: u8 = 0x02;
const MSG_OPEN: u8 = 0x03;
const MSG_OPEN_REPLY: u8 = 0x04;
const MSG_DATA_INCOMING: u8 = 0x05;
const MSG_DATA: u8 = 0x06;
const MSG_FORMAT_CHANGE: u8 = 0x07;
const WAVE_FORMAT_PCM: u16 = 1;
/// Version 1 of the protocol is enough for PCM.
const CLIENT_VERSION: u32 = 1;
/// Largest data packet sent at once (Windows usually asks for far less).
const MAX_PACKET_BYTES: usize = 64 * 1024;

/// The sound format Windows chose: always 16-bit PCM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MicFormat {
    pub rate: u32,
    pub channels: u16,
}

impl MicFormat {
    fn block_align(&self) -> usize {
        self.channels as usize * 2
    }

    fn encode(&self) -> [u8; 18] {
        let mut b = [0u8; 18];
        b[0..2].copy_from_slice(&WAVE_FORMAT_PCM.to_le_bytes());
        b[2..4].copy_from_slice(&self.channels.to_le_bytes());
        b[4..8].copy_from_slice(&self.rate.to_le_bytes());
        b[8..12].copy_from_slice(&(self.rate * self.block_align() as u32).to_le_bytes());
        b[12..14].copy_from_slice(&(self.block_align() as u16).to_le_bytes());
        b[14..16].copy_from_slice(&16u16.to_le_bytes());
        // cbSize (b[16..18]) stays 0: plain PCM has no extra data.
        b
    }
}

/// What the browser is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicEvent {
    /// A program in Windows is recording: send sound in this format.
    Opened(MicFormat),
    /// Recording stopped.
    Closed,
}

#[derive(Default)]
struct Shared {
    channel_id: Option<u32>,
    /// The formats offered back to Windows (indexes in open/format change refer to these).
    formats: Vec<MicFormat>,
    current: Option<MicFormat>,
    frames_per_packet: usize,
    /// Sound from the browser not yet sent (less than one packet).
    pending: Vec<u8>,
}

/// Bytes of one message.
struct Raw(Vec<u8>);

impl Encode for Raw {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.0.len());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "SNDIN_MESSAGE"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl DvcEncode for Raw {}

fn message(id: u8, body: &[u8]) -> DvcMessage {
    let mut v = Vec::with_capacity(1 + body.len());
    v.push(id);
    v.extend_from_slice(body);
    Box::new(Raw(v))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

/// The channel, handed to the RDP engine.
pub struct AudioInputClient {
    shared: Arc<Mutex<Shared>>,
    notify: Arc<dyn Fn(MicEvent) + Send + Sync>,
}

/// Remote's side: turns browser sound into channel messages.
#[derive(Clone)]
pub struct MicHandle {
    shared: Arc<Mutex<Shared>>,
}

/// A channel and its handle; `notify` hears when Windows opens or closes the microphone.
pub fn new(notify: Arc<dyn Fn(MicEvent) + Send + Sync>) -> (AudioInputClient, MicHandle) {
    let shared = Arc::new(Mutex::new(Shared::default()));
    (AudioInputClient { shared: shared.clone(), notify }, MicHandle { shared })
}

impl AudioInputClient {
    /// The same channel for another connection attempt (the state is shared with the handle).
    pub fn again(&self) -> Self {
        Self { shared: self.shared.clone(), notify: self.notify.clone() }
    }
}

ironrdp_core::impl_as_any!(AudioInputClient);

impl DvcProcessor for AudioInputClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        let mut s = self.shared.lock();
        *s = Shared { channel_id: Some(channel_id), ..Shared::default() };
        debug!("Microphone channel opened by the server ({})", channel_id);
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let Some(&id) = payload.first() else {
            return Ok(Vec::new());
        };
        match id {
            MSG_VERSION => Ok(vec![message(MSG_VERSION, &CLIENT_VERSION.to_le_bytes())]),
            MSG_FORMATS => {
                let offered = parse_formats(payload).ok_or_else(|| pdu_other_err!("AUDIO_INPUT formats", "malformed"))?;
                let usable: Vec<MicFormat> = offered.into_iter().flatten().collect();
                if usable.is_empty() {
                    warn!("Microphone: Windows offered no 16-bit PCM format");
                }
                let mut body = Vec::with_capacity(8 + usable.len() * 18);
                body.extend_from_slice(&(usable.len() as u32).to_le_bytes());
                body.extend_from_slice(&((1 + 8 + usable.len() * 18) as u32).to_le_bytes());
                for f in &usable {
                    body.extend_from_slice(&f.encode());
                }
                self.shared.lock().formats = usable;
                Ok(vec![message(MSG_FORMATS, &body)])
            }
            MSG_OPEN => {
                let frames = u32_at(payload, 1).unwrap_or(0) as usize;
                let index = u32_at(payload, 5).unwrap_or(u32::MAX);
                let format = {
                    let mut s = self.shared.lock();
                    let f = s.formats.get(index as usize).copied();
                    s.frames_per_packet = frames.max(1);
                    s.current = f;
                    s.pending.clear();
                    f
                };
                match format {
                    Some(f) => {
                        info!("Microphone: Windows started recording ({} Hz, {} channel(s))", f.rate, f.channels);
                        (self.notify)(MicEvent::Opened(f));
                        Ok(vec![
                            message(MSG_FORMAT_CHANGE, &index.to_le_bytes()),
                            message(MSG_OPEN_REPLY, &0u32.to_le_bytes()),
                        ])
                    }
                    None => {
                        warn!("Microphone: Windows asked for an unknown format {}", index);
                        // E_FAIL
                        Ok(vec![message(MSG_OPEN_REPLY, &0x8000_4005u32.to_le_bytes())])
                    }
                }
            }
            MSG_FORMAT_CHANGE => {
                let index = u32_at(payload, 1).unwrap_or(u32::MAX);
                let format = {
                    let mut s = self.shared.lock();
                    let f = s.formats.get(index as usize).copied();
                    if f.is_some() {
                        s.current = f;
                        s.pending.clear();
                    }
                    f
                };
                if let Some(f) = format {
                    (self.notify)(MicEvent::Opened(f));
                    return Ok(vec![message(MSG_FORMAT_CHANGE, &index.to_le_bytes())]);
                }
                Ok(Vec::new())
            }
            other => {
                debug!("Microphone: ignoring message {:#04x}", other);
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self, _channel_id: u32) {
        let was_open = {
            let mut s = self.shared.lock();
            let was = s.current.is_some();
            *s = Shared::default();
            was
        };
        if was_open {
            info!("Microphone: Windows stopped recording");
        }
        (self.notify)(MicEvent::Closed);
    }
}

/// The formats Windows offers; None for the ones Remote cannot send (not 16-bit PCM).
fn parse_formats(p: &[u8]) -> Option<Vec<Option<MicFormat>>> {
    let count = u32_at(p, 1)? as usize;
    let mut at = 9;
    let mut out = Vec::with_capacity(count.min(64));
    for _ in 0..count.min(256) {
        let tag = u16_at(p, at)?;
        let channels = u16_at(p, at + 2)?;
        let rate = u32_at(p, at + 4)?;
        let bits = u16_at(p, at + 14)?;
        let extra = u16_at(p, at + 16)? as usize;
        at += 18 + extra;
        let ok = tag == WAVE_FORMAT_PCM && bits == 16 && (channels == 1 || channels == 2) && (8000..=48000).contains(&rate);
        out.push(ok.then_some(MicFormat { rate, channels }));
    }
    Some(out)
}

impl MicHandle {
    /// The format to send in, while Windows records.
    pub fn format(&self) -> Option<MicFormat> {
        self.shared.lock().current
    }

    /// Sound from the browser (16-bit PCM in the current format): the messages to send,
    /// whole packets only; the rest waits for more sound. None while Windows is not recording.
    pub fn packets(&self, pcm: &[u8]) -> Option<(u32, Vec<DvcMessage>)> {
        let mut s = self.shared.lock();
        let (Some(id), Some(f)) = (s.channel_id, s.current) else {
            return None;
        };
        let packet = (s.frames_per_packet * f.block_align()).clamp(f.block_align(), MAX_PACKET_BYTES);
        s.pending.extend_from_slice(pcm);
        // Never let a stalled channel pile up sound: keep at most about a second.
        let cap = (f.rate as usize * f.block_align()).max(packet * 2);
        if s.pending.len() > cap {
            let drop = s.pending.len() - cap;
            let drop = drop - drop % f.block_align();
            s.pending.drain(..drop);
        }
        let mut msgs = Vec::new();
        while s.pending.len() >= packet {
            let chunk: Vec<u8> = s.pending.drain(..packet).collect();
            msgs.push(message(MSG_DATA_INCOMING, &[]));
            msgs.push(message(MSG_DATA, &chunk));
        }
        (!msgs.is_empty()).then_some((id, msgs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(m: &DvcMessage) -> Vec<u8> {
        let mut buf = vec![0u8; m.size()];
        m.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        buf
    }

    fn fmt(tag: u16, ch: u16, rate: u32, bits: u16) -> Vec<u8> {
        let align = ch * bits / 8;
        let mut v = Vec::new();
        v.extend_from_slice(&tag.to_le_bytes());
        v.extend_from_slice(&ch.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * align as u32).to_le_bytes());
        v.extend_from_slice(&align.to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v
    }

    #[test]
    fn the_whole_conversation() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let (mut ch, handle) = new(Arc::new(move |e| ev.lock().push(e)));
        assert!(ch.start(5).unwrap().is_empty());

        // Version
        let r = ch.process(5, &[MSG_VERSION, 2, 0, 0, 0]).unwrap();
        assert_eq!(bytes(&r[0]), vec![MSG_VERSION, 1, 0, 0, 0]);

        // Formats: an ADPCM one (skipped), 44.1 kHz stereo and 16 kHz mono PCM, 8-bit PCM (skipped)
        let offered = [fmt(2, 2, 44100, 4), fmt(1, 2, 44100, 16), fmt(1, 1, 16000, 16), fmt(1, 1, 8000, 8)];
        let mut p = vec![MSG_FORMATS];
        p.extend_from_slice(&4u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        for f in &offered {
            p.extend_from_slice(f);
        }
        let r = ch.process(5, &p).unwrap();
        let reply = bytes(&r[0]);
        assert_eq!(reply[0], MSG_FORMATS);
        assert_eq!(u32_at(&reply, 1), Some(2), "two usable formats offered back");
        assert_eq!(u32_at(&reply, 5), Some(1 + 8 + 36), "size of the whole message");
        assert_eq!(&reply[9..27], &offered[1][..]);
        assert_eq!(&reply[27..45], &offered[2][..]);

        // Open with our format 1 (16 kHz mono), 160 frames per packet
        let mut o = vec![MSG_OPEN];
        o.extend_from_slice(&160u32.to_le_bytes());
        o.extend_from_slice(&1u32.to_le_bytes());
        o.extend_from_slice(&offered[2]);
        let r = ch.process(5, &o).unwrap();
        assert_eq!(bytes(&r[0]), vec![MSG_FORMAT_CHANGE, 1, 0, 0, 0]);
        assert_eq!(bytes(&r[1]), vec![MSG_OPEN_REPLY, 0, 0, 0, 0]);
        assert_eq!(events.lock()[0], MicEvent::Opened(MicFormat { rate: 16000, channels: 1 }));

        // Sound: 160 frames x 2 bytes = 320 bytes per packet
        assert!(handle.packets(&[1u8; 200]).is_none(), "less than a packet waits");
        let (id, msgs) = handle.packets(&[2u8; 500]).unwrap();
        assert_eq!(id, 5);
        assert_eq!(msgs.len(), 4, "two packets: incoming + data each");
        assert_eq!(bytes(&msgs[0]), vec![MSG_DATA_INCOMING]);
        let data = bytes(&msgs[1]);
        assert_eq!((data[0], data.len()), (MSG_DATA, 321));
        assert_eq!(&data[1..201], &[1u8; 200][..]);

        // Format change to 44.1 kHz stereo
        let r = ch.process(5, &[MSG_FORMAT_CHANGE, 0, 0, 0, 0]).unwrap();
        assert_eq!(bytes(&r[0]), vec![MSG_FORMAT_CHANGE, 0, 0, 0, 0]);
        assert_eq!(handle.format(), Some(MicFormat { rate: 44100, channels: 2 }));

        ch.close(5);
        assert_eq!(*events.lock().last().unwrap(), MicEvent::Closed);
        assert!(handle.packets(&[0u8; 4000]).is_none(), "nothing is sent after the close");
    }

    #[test]
    fn an_unknown_format_is_refused() {
        let (mut ch, _h) = new(Arc::new(|_| {}));
        ch.start(1).unwrap();
        let mut o = vec![MSG_OPEN];
        o.extend_from_slice(&160u32.to_le_bytes());
        o.extend_from_slice(&9u32.to_le_bytes());
        let r = ch.process(1, &o).unwrap();
        assert_eq!(bytes(&r[0])[0], MSG_OPEN_REPLY);
        assert_ne!(u32_at(&bytes(&r[0]), 1), Some(0));
    }

    #[test]
    fn a_stalled_channel_keeps_at_most_a_second() {
        let (mut ch, h) = new(Arc::new(|_| {}));
        ch.start(1).unwrap();
        {
            let mut s = ch.shared.lock();
            s.formats = vec![MicFormat { rate: 8000, channels: 1 }];
            s.current = Some(s.formats[0]);
            s.frames_per_packet = 100_000; // larger than the cap: nothing goes out
        }
        let _ = h.packets(&vec![0u8; 100_000]);
        assert!(ch.shared.lock().pending.len() <= MAX_PACKET_BYTES * 2);
    }
}
