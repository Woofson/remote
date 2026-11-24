//! RDP client engine, adapted from `ironrdp-client` 0.1.0.
//!
//! Copyright (c) Devolutions Inc. and the IronRDP contributors.
//! Licensed under MIT OR Apache-2.0; see LICENSE-MIT in this directory.
//! Upstream: https://github.com/Devolutions/IronRDP (crates/ironrdp-client)
//!
//! Why it lives here: the published crate hard-codes the RDP performance flags
//! (wallpaper, full-window drag, animations, themes, font smoothing) and never
//! advertises audio playback unless its own native-audio backend is compiled in.
//! Remote needs both under its own control. See docs/rdp-client.md for the exact
//! changes, how to sync with upstream, and the patch offered upstream.
//!
//! Optional upstream features are resolved at copy time: `clipboard`, `rdpdr` and
//! `rustls` became `all()` (on); every other feature became `any()` (off).

#![allow(clippy::arithmetic_side_effects)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(dead_code)]

pub mod config;
pub mod rdp;

#[cfg(all(windows, all()))]
mod clipboard;

mod ws;
