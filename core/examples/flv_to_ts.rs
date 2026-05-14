//! FLV (HEVC+AAC) → MPEG-TS pipe. Lets any TS-aware player (mpv, ffprobe,
//! libVLC) consume Streamax dashcam streams with audio.
//!
//! Use: curl -sk URL | cargo run --release --example flv_to_ts | mpv -

use std::io::{Read, Write};
use streamax_core::{Demuxer, Event, TsMuxer};

fn main() {
    let mut d = Demuxer::new();
    let mut m = TsMuxer::new();
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let mut buf = vec![0u8; 64 * 1024];

    loop {
        let n = match input.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => { eprintln!("stdin: {e}"); break; }
        };
        d.append(&buf[..n]);

        while let Some(ev) = d.next_event() {
            match ev {
                Event::VideoConfig { annex_b, width, height } => {
                    eprintln!("VideoConfig: {}x{}, {} bytes", width, height, annex_b.len());
                    m.push_video_config(&annex_b);
                }
                Event::VideoFrame { annex_b, pts_ms, is_keyframe } => {
                    m.push_video_frame(&annex_b, pts_ms, is_keyframe);
                }
                Event::AudioConfig { sample_rate, channels, object_type, .. } => {
                    eprintln!("AudioConfig: {}Hz {}ch (object_type={})", sample_rate, channels, object_type);
                    m.push_audio_config(sample_rate, channels, object_type);
                }
                Event::AudioFrame { data, pts_ms } => {
                    m.push_audio_frame(&data, pts_ms);
                }
                Event::Error(msg) => { eprintln!("ERROR: {msg}"); return; }
            }
        }

        let chunk = m.drain();
        if !chunk.is_empty() {
            if output.write_all(&chunk).is_err() { break; }
        }
    }
}
