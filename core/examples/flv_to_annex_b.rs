//! Reads FLV from stdin, writes HEVC Annex-B to stdout. Mirrors flv_hevc.py.
//!
//! Use: curl -sk URL | cargo run --release --example flv_to_annex_b | mpv ...

use std::io::{Read, Write};
use streamax_core::{Demuxer, Event};

fn main() {
    let mut d = Demuxer::new();
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let mut buf = vec![0u8; 64 * 1024];
    let mut stats = (0usize, 0usize, 0usize, 0usize); // cfg, frames, audio_cfg, audio_frames

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
                    output.write_all(&annex_b).ok();
                    stats.0 += 1;
                }
                Event::VideoFrame { annex_b, .. } => {
                    output.write_all(&annex_b).ok();
                    stats.1 += 1;
                }
                Event::AudioConfig { sample_rate, channels, .. } => {
                    eprintln!("AudioConfig: {}Hz {}ch", sample_rate, channels);
                    stats.2 += 1;
                }
                Event::AudioFrame { .. } => {
                    stats.3 += 1;
                }
                Event::Error(msg) => {
                    eprintln!("ERROR: {msg}");
                    return;
                }
            }
        }
    }
    eprintln!("done: {} video cfg, {} frames, {} audio cfg, {} audio frames",
        stats.0, stats.1, stats.2, stats.3);
}
