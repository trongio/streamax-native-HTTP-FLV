//! Streamax FLV demuxer — single source of truth.
//!
//! The HTTP-FLV streams produced by Streamax (and similar Chinese MDVR
//! vendors) use a non-standard FLV variant: `codec_id = 12 = HEVC` directly
//! in the video tag header, rather than the enhanced-RTMP FOURCC method.
//! Mainline FFmpeg, libVLC, ExoPlayer and friends all fail (or crash) on
//! this. We do it ourselves — pull style, no callbacks, easy to FFI.
//!
//! Pull API:
//!
//!   let mut d = Demuxer::new();
//!   d.append(bytes);
//!   while let Some(ev) = d.next_event() { handle(ev); }
//!
//! For C / Swift / Kotlin / C# bindings, see [`ffi`].

#![allow(clippy::missing_safety_doc)]

mod aac;
pub mod ffi;
mod flv;
mod hevc;
pub mod mpegts;

pub use flv::{Demuxer, Event};
pub use mpegts::Muxer as TsMuxer;
