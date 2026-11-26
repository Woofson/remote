# RDP client engine

Remote's RDP connections run on an RDP client engine adapted from the
[`ironrdp-client`](https://crates.io/crates/ironrdp-client) crate (version 0.1.0,
part of [IronRDP](https://github.com/Devolutions/IronRDP) by Devolutions and
contributors, MIT OR Apache-2.0). The code lives in `src/protocols/rdp_client/`,
with the original licence notice in `LICENSE-MIT` next to it.

## Why it is part of Remote

The published crate has two limits that Remote cannot work around from outside:

1. **Performance flags are hard-coded.** The crate always sends its default flags
   (wallpaper shown, outline-only window dragging, no menu animations, font
   smoothing on). Remote's per-connection settings for wallpaper, window contents
   while dragging, menu animations, themes and font smoothing were ignored (#28).
2. **Audio is never advertised** unless the crate's own `sound` feature is built in.
   That feature plays sound on the machine running Remote (through the system
   sound stack), not in the user's browser, and pulls in native audio libraries.
   Remote needs to receive the audio itself and forward it to the browser (#25).

Patching the crate only for local builds would break `cargo install remoted`, so
the engine is carried inside the `remoted` crate instead.

## What differs from upstream

- **Feature flags resolved at copy time.** Upstream `#[cfg(feature = "...")]`
  markers were replaced mechanically: `clipboard`, `rdpdr` and `rustls` became
  `all()` (always on); `sound`, `gateway`, `dvc-pipe-proxy`, `dvc-com-plugin`,
  `smartcard`, `qoi`, `qoiz` and `native-tls` became `any()` (always off). The
  code is otherwise unchanged, which keeps upstream diffs small.
- **Module paths**: `crate::config`, `crate::rdp`, `crate::ws` and
  `crate::clipboard` became `super::...`; `lib.rs` became `mod.rs`.
- **Two additions** (the functional change, identical to the upstream patch):
  - `ConfigBuilder::with_performance_flags(flags)` sets the Client Info PDU
    performance flags; the default stays upstream's default.
  - `ConfigBuilder::with_external_audio(true)` advertises audio playback while the
    caller supplies its own RDPSND channel through `with_static_channel`. Remote
    uses this to forward PCM audio to the browser (`WebAudioBackend` in
    `src/protocols/rdp.rs`).
- **Later Remote-only additions** (not in the upstream patch; re-apply after a sync):
  - `ConfigBuilder::with_keyboard_layout(id)` sends the Windows keyboard layout
    at connect instead of `0` (#6).
  - `RdpInputEvent::SuppressOutput(bool)` pauses and resumes screen updates with
    the Suppress Output PDU while a session's tab or pane is hidden (#7).
  - `ConfigBuilder::with_audio_capture(true)` sets INFO_AUDIOCAPTURE in the Client
    Info PDU, which Windows needs before it opens the microphone channel (#37). The
    connector has no option for this flag, so `audio_capture.rs` wraps the TLS
    stream and sets that one bit on the outgoing Client Info PDU (once, no length
    change; covered by a test that decodes the PDU). The microphone channel itself
    (AUDIO_INPUT, MS-RDPEAI) is Remote code in `src/protocols/rdp_mic.rs`, attached
    with `with_dvc`.

Known upstream issue: during the Deactivation-Reactivation Sequence (Windows runs
one after every resize) `active_session` rebuilds the fast-path processor with
`bulk_decompressor: None`, and `ironrdp-session` offers no way to keep the old
one. Windows continues its compression history, so with bulk compression on, the
first update after a resize fails ("NCRUSH: history offset too small"). Remote
therefore connects with `with_compression(false)` (see `src/protocols/rdp.rs`).
Worth reporting upstream; compression can be turned back on once fixed there.

## Syncing with a new upstream release

1. Copy `config.rs`, `rdp.rs`, `ws.rs` and `clipboard.rs` from the new
   `ironrdp-client` source into `src/protocols/rdp_client/`.
2. Repeat the feature-flag and module-path replacements listed above.
3. Re-apply `docs/upstream/ironrdp-client-performance-flags-external-audio.patch`
   (`patch -p1` from a directory where `src/` is the crate source), unless
   upstream has merged an equivalent change, and the later Remote-only additions.
4. Check the crate's dependency list against the "RDP client engine" block in
   `Cargo.toml`, then build and test an RDP session with sound and drive
   redirection.

## Going back to the published crate

The patch in `docs/upstream/` applies to a clean `ironrdp-client` 0.1.0 checkout
and has been compile-checked there. If IronRDP accepts it (or adds equivalent
options), Remote can switch back to the published crate: remove
`src/protocols/rdp_client/`, restore the `ironrdp-client` dependency, and point
the two `use` lines at the top of `src/protocols/rdp.rs` back to it.
