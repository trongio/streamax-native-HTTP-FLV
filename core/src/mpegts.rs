//! Minimal MPEG-TS muxer for HEVC + AAC. Output is consumable by libVLC,
//! ffmpeg, mpv and any standard TS demuxer.
//!
//! Scope: live streams, one video + one audio elementary stream, no PCR
//! (PTS used directly), no subtitles, no PMT versioning. Enough to feed a
//! native player without writing a media framework.
//!
//! References: ISO/IEC 13818-1 (TS), ISO/IEC 14496-3 (ADTS), ISO/IEC 23008-2 (HEVC).

const TS_PACKET_SIZE: usize = 188;
const SYNC_BYTE: u8 = 0x47;

const PID_PAT: u16 = 0x0000;
const PID_PMT: u16 = 0x1000;
const PID_VIDEO: u16 = 0x0100;
const PID_AUDIO: u16 = 0x0101;

const STREAM_ID_VIDEO: u8 = 0xE0;
const STREAM_ID_AUDIO: u8 = 0xC0;

const STREAM_TYPE_HEVC: u8 = 0x24;
const STREAM_TYPE_AAC: u8 = 0x0F;

const TABLES_INTERVAL_MS: u32 = 1000;

pub struct Muxer {
    cc: [u8; 4], // continuity counters: 0=pat, 1=pmt, 2=video, 3=audio
    out: Vec<u8>,
    video_csd_annex_b: Vec<u8>, // VPS+SPS+PPS, prepended to each keyframe
    aac_config: Option<AacConfig>,
    last_tables_pts: u32,
    has_video_config: bool,
    has_audio_config: bool,
}

#[derive(Clone, Copy)]
struct AacConfig {
    sample_rate_index: u8,
    channels: u8,
    profile_minus1: u8, // ADTS profile = object_type - 1
}

impl Default for Muxer {
    fn default() -> Self {
        Self::new()
    }
}

impl Muxer {
    pub fn new() -> Self {
        Self {
            cc: [0; 4],
            out: Vec::with_capacity(64 * 1024),
            video_csd_annex_b: Vec::new(),
            aac_config: None,
            last_tables_pts: 0,
            has_video_config: false,
            has_audio_config: false,
        }
    }

    pub fn push_video_config(&mut self, annex_b: &[u8]) {
        self.video_csd_annex_b = annex_b.to_vec();
        self.has_video_config = true;
    }

    pub fn push_video_frame(&mut self, annex_b: &[u8], pts_ms: u32, is_keyframe: bool) {
        if !self.has_video_config {
            return;
        }
        self.maybe_emit_tables(pts_ms);

        // Prepend VPS/SPS/PPS on every keyframe for clean tune-in.
        let mut nalus = Vec::with_capacity(self.video_csd_annex_b.len() + annex_b.len() + 16);
        if is_keyframe {
            nalus.extend_from_slice(&self.video_csd_annex_b);
        }
        // HEVC stream-format requires an Access Unit Delimiter (NAL type 35)
        // ahead of each AU to keep some demuxers happy. Cheap to add.
        nalus.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x46, 0x01, 0x10]);
        nalus.extend_from_slice(annex_b);

        let pes = build_pes(STREAM_ID_VIDEO, pts_ms, &nalus, /* unbounded */ true);
        self.write_pes_into_ts(PID_VIDEO, 2, &pes, /* random_access */ is_keyframe);
    }

    pub fn push_audio_config(&mut self, sample_rate: u32, channels: u8, object_type: u8) {
        let sr_index = sample_rate_index(sample_rate).unwrap_or(4); // 44.1k default
        self.aac_config = Some(AacConfig {
            sample_rate_index: sr_index,
            channels: channels.max(1).min(7),
            profile_minus1: object_type.saturating_sub(1).max(0),
        });
        self.has_audio_config = true;
    }

    pub fn push_audio_frame(&mut self, raw_aac: &[u8], pts_ms: u32) {
        let Some(cfg) = self.aac_config else {
            return;
        };
        self.maybe_emit_tables(pts_ms);

        let adts = wrap_adts(raw_aac, cfg);
        let pes = build_pes(STREAM_ID_AUDIO, pts_ms, &adts, /* unbounded */ false);
        self.write_pes_into_ts(PID_AUDIO, 3, &pes, /* random_access */ true);
    }

    /// Drain any pending TS bytes.
    pub fn drain(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    fn maybe_emit_tables(&mut self, pts_ms: u32) {
        let due = self.last_tables_pts == 0 || pts_ms.saturating_sub(self.last_tables_pts) >= TABLES_INTERVAL_MS;
        if !(due && self.has_video_config) {
            return;
        }
        self.write_pat();
        self.write_pmt();
        self.last_tables_pts = pts_ms;
    }

    fn write_pat(&mut self) {
        let mut payload = Vec::with_capacity(20);
        payload.push(0x00); // pointer_field
        // PAT section
        let section = build_pat_section();
        payload.extend_from_slice(&section);
        self.write_psi_packet(PID_PAT, 0, &payload);
    }

    fn write_pmt(&mut self) {
        let mut payload = Vec::with_capacity(40);
        payload.push(0x00); // pointer_field
        let section = build_pmt_section(self.has_audio_config);
        payload.extend_from_slice(&section);
        self.write_psi_packet(PID_PMT, 1, &payload);
    }

    fn write_psi_packet(&mut self, pid: u16, cc_idx: usize, payload: &[u8]) {
        // PSI fits in one TS packet (we don't paginate; PAT/PMT are tiny).
        let mut pkt = [0xFFu8; TS_PACKET_SIZE];
        pkt[0] = SYNC_BYTE;
        // PUSI=1, TEI=0, transport_priority=0, PID hi 5 bits
        pkt[1] = 0x40 | ((pid >> 8) as u8 & 0x1F);
        pkt[2] = pid as u8;
        // TSC=00, AFC=01 (payload only), CC
        pkt[3] = 0x10 | (self.cc[cc_idx] & 0x0F);
        self.cc[cc_idx] = (self.cc[cc_idx] + 1) & 0x0F;

        let n = payload.len().min(TS_PACKET_SIZE - 4);
        pkt[4..4 + n].copy_from_slice(&payload[..n]);
        // remaining bytes are 0xFF stuffing (already initialized)
        self.out.extend_from_slice(&pkt);
    }

    fn write_pes_into_ts(&mut self, pid: u16, cc_idx: usize, pes: &[u8], random_access: bool) {
        let mut offset = 0;
        let mut first = true;

        while offset < pes.len() {
            let mut pkt = [0u8; TS_PACKET_SIZE];
            pkt[0] = SYNC_BYTE;
            // PUSI set on first packet of a PES
            let pusi = if first { 0x40 } else { 0 };
            pkt[1] = pusi | ((pid >> 8) as u8 & 0x1F);
            pkt[2] = pid as u8;

            // Decide whether we need an adaptation field
            let remaining = pes.len() - offset;
            let mut af_len = 0usize;
            let mut af_flags = 0u8;

            // Adaptation field on first packet of a keyframe (random_access_indicator)
            // and as padding when remaining payload < 184.
            if first && random_access {
                af_flags |= 0x40;
                af_len = 1;
            }
            let payload_capacity = TS_PACKET_SIZE - 4 - if af_len > 0 { af_len + 1 } else { 0 };
            if remaining < payload_capacity {
                // Pad with adaptation field stuffing.
                let need_pad = payload_capacity - remaining;
                af_len += need_pad;
                if af_flags == 0 {
                    // need at least the flags byte
                    af_len = af_len.max(1);
                }
            }

            let afc: u8;
            let payload_start;

            if af_len > 0 {
                afc = 0x30; // adaptation field + payload
                pkt[4] = (af_len as u8) - 0; // length field is length of AF *after* the length byte
                // length encoding: length value = af_total_bytes - 1 (the length byte itself is excluded)
                // We treat af_len as "bytes including flags + stuffing"; ISO requires the length byte
                // to indicate everything after it. So len_field = af_len.
                pkt[4] = af_len as u8;
                pkt[5] = af_flags;
                // Stuffing (already 0); ISO says stuffing should be 0xFF but most demuxers tolerate 0.
                for b in pkt.iter_mut().skip(6).take(af_len.saturating_sub(1)) {
                    *b = 0xFF;
                }
                payload_start = 5 + af_len;
            } else {
                afc = 0x10; // payload only
                payload_start = 4;
            }

            pkt[3] = afc | (self.cc[cc_idx] & 0x0F);
            self.cc[cc_idx] = (self.cc[cc_idx] + 1) & 0x0F;

            let payload_room = TS_PACKET_SIZE - payload_start;
            let take = remaining.min(payload_room);
            pkt[payload_start..payload_start + take].copy_from_slice(&pes[offset..offset + take]);
            offset += take;
            first = false;

            self.out.extend_from_slice(&pkt);
        }
    }
}

fn build_pat_section() -> Vec<u8> {
    let mut s = Vec::with_capacity(17);
    s.push(0x00); // table_id = PAT
    // section_syntax_indicator(1)=1, '0'(1), reserved(2)=11, length(12)
    // length covers from after this 2-byte field through CRC32
    let section_body = {
        let mut b = Vec::new();
        b.extend_from_slice(&[0x00, 0x01]); // transport_stream_id = 1
        b.push(0xC1); // reserved(2)=11, version(5)=0, current_next(1)=1
        b.push(0x00); // section_number
        b.push(0x00); // last_section_number
        b.extend_from_slice(&[0x00, 0x01]); // program_number = 1
        b.extend_from_slice(&[0xE0 | ((PID_PMT >> 8) as u8 & 0x1F), PID_PMT as u8]); // reserved(3)=111 + PMT PID
        b
    };
    let section_len = section_body.len() + 4; // body + CRC32
    s.push(0xB0 | ((section_len >> 8) as u8 & 0x0F));
    s.push(section_len as u8);
    s.extend_from_slice(&section_body);
    let crc = crc32_mpeg2(&s[..s.len()]);
    s.extend_from_slice(&crc.to_be_bytes());
    s
}

fn build_pmt_section(has_audio: bool) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&[0x00, 0x01]); // program_number = 1
    body.push(0xC1); // reserved + version + current_next
    body.push(0x00); // section_number
    body.push(0x00); // last_section_number
    body.extend_from_slice(&[0xE0 | ((PID_VIDEO >> 8) as u8 & 0x1F), PID_VIDEO as u8]); // PCR PID = video PID
    body.extend_from_slice(&[0xF0, 0x00]); // program_info_length = 0

    // ES: video
    body.push(STREAM_TYPE_HEVC);
    body.extend_from_slice(&[0xE0 | ((PID_VIDEO >> 8) as u8 & 0x1F), PID_VIDEO as u8]);
    body.extend_from_slice(&[0xF0, 0x00]); // ES_info_length = 0
    if has_audio {
        body.push(STREAM_TYPE_AAC);
        body.extend_from_slice(&[0xE0 | ((PID_AUDIO >> 8) as u8 & 0x1F), PID_AUDIO as u8]);
        body.extend_from_slice(&[0xF0, 0x00]);
    }

    let mut s = Vec::with_capacity(body.len() + 12);
    s.push(0x02); // table_id
    let section_len = body.len() + 4; // body + CRC32
    s.push(0xB0 | ((section_len >> 8) as u8 & 0x0F));
    s.push(section_len as u8);
    s.extend_from_slice(&body);
    let crc = crc32_mpeg2(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    s
}

/// Build a PES packet with the 33-bit PTS field. `unbounded` controls
/// whether PES_packet_length is set to 0 (for video) or to the actual length.
fn build_pes(stream_id: u8, pts_ms: u32, payload: &[u8], unbounded: bool) -> Vec<u8> {
    // PTS is at 90kHz.
    let pts_90k = (pts_ms as u64) * 90;

    let mut h = Vec::with_capacity(14 + payload.len());
    h.extend_from_slice(&[0x00, 0x00, 0x01]); // start code prefix
    h.push(stream_id);

    // header below this point: 3 bytes optional PES header + 5 bytes PTS = 8 bytes header data
    // Plus 0 stuffing.
    let header_data_len: u16 = 5; // PTS only
    let pes_payload_len = 3 + header_data_len as u16 + payload.len() as u16;

    let length_field: u16 = if unbounded { 0 } else { pes_payload_len };
    h.extend_from_slice(&length_field.to_be_bytes());

    h.push(0x80); // '10' + flags
    h.push(0x80); // PTS_DTS_flags = '10' (PTS only)
    h.push(header_data_len as u8);
    h.extend_from_slice(&encode_pts(0x02, pts_90k)); // 5 bytes PTS

    h.extend_from_slice(payload);
    h
}

fn encode_pts(marker_high: u8, pts: u64) -> [u8; 5] {
    let mut b = [0u8; 5];
    // marker(4) | PTS[32..30] | marker(1) | PTS[29..15] | marker(1) | PTS[14..0] | marker(1)
    b[0] = (marker_high << 4) | (((pts >> 29) & 0x0E) as u8) | 0x01;
    b[1] = ((pts >> 22) & 0xFF) as u8;
    b[2] = (((pts >> 14) & 0xFE) as u8) | 0x01;
    b[3] = ((pts >> 7) & 0xFF) as u8;
    b[4] = (((pts << 1) & 0xFE) as u8) | 0x01;
    b
}

/// ADTS-wrap a raw AAC frame. ISO/IEC 14496-3 §1.6.2.1 / §1.A.2.2.
fn wrap_adts(raw_aac: &[u8], cfg: AacConfig) -> Vec<u8> {
    let frame_len = raw_aac.len() + 7;
    let mut out = Vec::with_capacity(frame_len);
    out.push(0xFF); // syncword high
    // syncword low(4) | ID(1) | layer(2) | protection_absent(1)
    out.push(0xF1);
    // profile(2) | sf_index(4) | private(1) | ch_high(1)
    out.push(((cfg.profile_minus1 & 0x03) << 6)
        | ((cfg.sample_rate_index & 0x0F) << 2)
        | (((cfg.channels & 0x04) >> 2) & 0x01));
    // ch_low(2) | original(1) | home(1) | copyright_id_bit(1) | copyright_id_start(1) | frame_len[12..11]
    out.push(((cfg.channels & 0x03) << 6) | ((frame_len >> 11) as u8 & 0x03));
    out.push((frame_len >> 3) as u8);
    out.push(((frame_len << 5) as u8 & 0xE0) | 0x1F); // frame_len[2..0] | buffer fullness high (5 bits, set to 0x1F)
    out.push(0xFC); // buffer fullness low + frames_in_block(0)
    out.extend_from_slice(raw_aac);
    out
}

fn sample_rate_index(sr: u32) -> Option<u8> {
    Some(match sr {
        96000 => 0, 88200 => 1, 64000 => 2, 48000 => 3,
        44100 => 4, 32000 => 5, 24000 => 6, 22050 => 7,
        16000 => 8, 12000 => 9, 11025 => 10, 8000 => 11, 7350 => 12,
        _ => return None,
    })
}

/// CRC-32/MPEG-2: polynomial 0x04C11DB7, init 0xFFFFFFFF, no reflection, no xorout.
fn crc32_mpeg2(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C11DB7 } else { crc << 1 };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pat_section_starts_with_table_id() {
        let s = build_pat_section();
        assert_eq!(s[0], 0x00); // PAT table_id
        // 1 byte table_id + 2 length + 9 body (TS_ID..PID) + 4 CRC = 16
        assert_eq!(s.len(), 16);
    }

    #[test]
    fn muxer_emits_packets() {
        let mut m = Muxer::new();
        m.push_video_config(&[0x00, 0x00, 0x00, 0x01, 0x40, 0x01]);
        m.push_video_frame(&[0x00, 0x00, 0x00, 0x01, 0x26, 0x01, 0x00, 0x00], 0, true);
        let out = m.drain();
        assert!(!out.is_empty(), "muxer produced no output");
        assert_eq!(out[0], SYNC_BYTE);
        assert_eq!(out.len() % TS_PACKET_SIZE, 0, "TS output is not packet-aligned");
    }

    #[test]
    fn adts_header_size() {
        let cfg = AacConfig { sample_rate_index: 11, channels: 1, profile_minus1: 1 };
        let out = wrap_adts(&[0xAA; 100], cfg);
        assert_eq!(out.len(), 107);
        assert_eq!(out[0], 0xFF);
        assert_eq!(out[1] & 0xF0, 0xF0); // syncword
    }
}
