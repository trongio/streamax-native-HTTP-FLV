use crate::aac;
use crate::hevc;
use std::collections::VecDeque;

const START_CODE: [u8; 4] = [0, 0, 0, 1];

#[derive(Debug)]
pub enum Event {
    /// HEVC parameter sets (VPS+SPS+PPS) concatenated in Annex-B form,
    /// plus decoded resolution from SPS. Emit once before video frames.
    VideoConfig {
        annex_b: Vec<u8>,
        width: u32,
        height: u32,
    },
    /// One frame's worth of HEVC NALUs in Annex-B form.
    VideoFrame {
        annex_b: Vec<u8>,
        pts_ms: u32,
        is_keyframe: bool,
    },
    /// AudioSpecificConfig (2 bytes typical). Emit once before audio frames.
    AudioConfig {
        config: Vec<u8>,
        sample_rate: u32,
        channels: u8,
        object_type: u8,
    },
    /// One AAC raw frame (no ADTS header). PTS in milliseconds.
    AudioFrame {
        data: Vec<u8>,
        pts_ms: u32,
    },
    /// Demuxer hit unrecoverable error. After this, [`Demuxer`] is poisoned.
    Error(String),
}

pub struct Demuxer {
    pending: Vec<u8>,
    cursor: usize,
    header_parsed: bool,
    poisoned: bool,
    events: VecDeque<Event>,
}

impl Default for Demuxer {
    fn default() -> Self {
        Self::new()
    }
}

impl Demuxer {
    pub fn new() -> Self {
        Self {
            pending: Vec::with_capacity(128 * 1024),
            cursor: 0,
            header_parsed: false,
            poisoned: false,
            events: VecDeque::new(),
        }
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.cursor = 0;
        self.header_parsed = false;
        self.poisoned = false;
        self.events.clear();
    }

    /// Push more bytes. Parses what it can; queues events.
    pub fn append(&mut self, chunk: &[u8]) {
        if self.poisoned || chunk.is_empty() {
            return;
        }
        self.pending.extend_from_slice(chunk);
        self.parse();
        // Compact occasionally so memory doesn't grow unbounded.
        if self.cursor > 256 * 1024 {
            self.pending.drain(..self.cursor);
            self.cursor = 0;
        }
    }

    /// Pop the next event, if any. None means "feed more bytes."
    pub fn next_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    fn parse(&mut self) {
        if !self.header_parsed {
            if self.pending.len() - self.cursor < 13 {
                return;
            }
            let h = &self.pending[self.cursor..self.cursor + 3];
            if h != b"FLV" {
                self.poisoned = true;
                self.events.push_back(Event::Error("not an FLV stream".into()));
                return;
            }
            self.cursor += 13; // 9-byte header + 4-byte PreviousTagSize0
            self.header_parsed = true;
        }

        loop {
            let buf = &self.pending[self.cursor..];
            if buf.len() < 11 {
                return;
            }
            let tag_type = buf[0] & 0x1F;
            let data_size = ((buf[1] as usize) << 16) | ((buf[2] as usize) << 8) | (buf[3] as usize);
            let ts = ((buf[7] as u32) << 24)
                | ((buf[4] as u32) << 16)
                | ((buf[5] as u32) << 8)
                | (buf[6] as u32);

            let total = 11 + data_size + 4;
            if buf.len() < total {
                return;
            }

            // Detach the body from the shared borrow of self.pending so the
            // handlers can take &mut self for queueing events.
            let body = buf[11..11 + data_size].to_vec();
            self.cursor += total;

            match tag_type {
                9 => Self::handle_video(&mut self.events, &mut self.poisoned, &body, ts),
                8 => Self::handle_audio(&mut self.events, &body, ts),
                _ => {}
            }

            if self.poisoned {
                return;
            }
        }
    }

    fn handle_video(events: &mut VecDeque<Event>, poisoned: &mut bool, body: &[u8], ts: u32) {
        if body.len() < 5 {
            return;
        }
        let codec_id = body[0] & 0x0F;
        if codec_id != 12 {
            if codec_id == 7 {
                events.push_back(Event::Error(
                    "stream is H.264/AVC; this build is HEVC-only".into(),
                ));
                *poisoned = true;
            }
            return;
        }
        let is_keyframe = ((body[0] >> 4) & 0x0F) == 1;
        let packet_type = body[1];
        let payload = &body[5..];
        match packet_type {
            0 => {
                if let Some((annex_b, width, height)) = parse_hvcc(payload) {
                    events.push_back(Event::VideoConfig {
                        annex_b,
                        width,
                        height,
                    });
                }
            }
            1 => {
                let annex_b = avcc_to_annex_b(payload);
                if !annex_b.is_empty() {
                    events.push_back(Event::VideoFrame {
                        annex_b,
                        pts_ms: ts,
                        is_keyframe,
                    });
                }
            }
            _ => {}
        }
    }

    fn handle_audio(events: &mut VecDeque<Event>, body: &[u8], ts: u32) {
        if body.len() < 2 {
            return;
        }
        // First byte: high nibble = codec (10 = AAC), low nibble = sample rate index etc.
        let codec = (body[0] >> 4) & 0x0F;
        if codec != 10 {
            return;
        }
        let aac_packet_type = body[1];
        let payload = &body[2..];
        if payload.is_empty() {
            return;
        }
        match aac_packet_type {
            0 => {
                if let Some(cfg) = aac::parse_audio_specific_config(payload) {
                    events.push_back(Event::AudioConfig {
                        config: payload.to_vec(),
                        sample_rate: cfg.sample_rate,
                        channels: cfg.channels,
                        object_type: cfg.object_type,
                    });
                }
            }
            1 => {
                events.push_back(Event::AudioFrame {
                    data: payload.to_vec(),
                    pts_ms: ts,
                });
            }
            _ => {}
        }
    }
}

fn avcc_to_annex_b(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    let mut p = 0;
    while p + 4 <= data.len() {
        let len = u32::from_be_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]) as usize;
        p += 4;
        if len == 0 || p + len > data.len() {
            break;
        }
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(&data[p..p + len]);
        p += len;
    }
    out
}

fn parse_hvcc(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    if data.len() < 23 || data[0] != 1 {
        return None;
    }
    let mut p = 22;
    if p >= data.len() {
        return None;
    }
    let num_arrays = data[p] as usize;
    p += 1;

    let mut annex_b = Vec::with_capacity(256);
    let mut sps_nalu: Option<&[u8]> = None;

    for _ in 0..num_arrays {
        if p + 3 > data.len() {
            return None;
        }
        let nal_type = data[p] & 0x3F;
        p += 1;
        let num_nalus = u16::from_be_bytes([data[p], data[p + 1]]) as usize;
        p += 2;
        for _ in 0..num_nalus {
            if p + 2 > data.len() {
                return None;
            }
            let len = u16::from_be_bytes([data[p], data[p + 1]]) as usize;
            p += 2;
            if p + len > data.len() || len == 0 {
                return None;
            }
            let nalu = &data[p..p + len];
            p += len;
            annex_b.extend_from_slice(&START_CODE);
            annex_b.extend_from_slice(nalu);
            if nal_type == 33 {
                sps_nalu = Some(nalu);
            }
        }
    }

    let (w, h) = sps_nalu.and_then(hevc::sps_dimensions).unwrap_or((0, 0));
    Some((annex_b, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_flv() {
        let mut d = Demuxer::new();
        d.append(b"NOTFLV..........");
        assert!(matches!(d.next_event(), Some(Event::Error(_))));
    }

    #[test]
    fn accepts_minimal_header() {
        let mut d = Demuxer::new();
        d.append(&[
            b'F', b'L', b'V', 1, 5, 0, 0, 0, 9, // header
            0, 0, 0, 0, // PreviousTagSize0
        ]);
        assert!(d.next_event().is_none());
        assert!(d.header_parsed);
    }
}
