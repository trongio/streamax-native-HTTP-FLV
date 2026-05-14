//! AAC AudioSpecificConfig parser. Spec: ISO/IEC 14496-3 §1.6.2.1.

pub struct AacConfig {
    pub object_type: u8,
    pub sample_rate: u32,
    pub channels: u8,
}

const AAC_SAMPLE_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

pub fn parse_audio_specific_config(data: &[u8]) -> Option<AacConfig> {
    if data.len() < 2 {
        return None;
    }
    // 5 bits object type
    let mut object_type = (data[0] >> 3) & 0x1F;
    let mut bit_pos = 5;

    if object_type == 31 {
        // 6-bit extension
        let v = read_bits(data, bit_pos, 6)?;
        bit_pos += 6;
        object_type = (v + 32) as u8;
    }

    // 4 bits sampling frequency index
    let sf_index = read_bits(data, bit_pos, 4)? as usize;
    bit_pos += 4;
    let sample_rate = if sf_index == 0x0F {
        // 24-bit explicit sample rate
        let sr = read_bits(data, bit_pos, 24)?;
        bit_pos += 24;
        sr
    } else if sf_index < AAC_SAMPLE_RATES.len() {
        AAC_SAMPLE_RATES[sf_index]
    } else {
        return None;
    };

    // 4 bits channel configuration
    let channel_config = read_bits(data, bit_pos, 4)? as u8;
    let channels = if channel_config == 7 { 8 } else { channel_config };

    Some(AacConfig {
        object_type,
        sample_rate,
        channels,
    })
}

fn read_bits(data: &[u8], bit_offset: usize, n: usize) -> Option<u32> {
    if n == 0 {
        return Some(0);
    }
    if n > 32 {
        return None;
    }
    let mut v = 0u64;
    for i in 0..n {
        let pos = bit_offset + i;
        let byte = *data.get(pos / 8)?;
        let bit = (byte >> (7 - (pos % 8))) & 1;
        v = (v << 1) | (bit as u64);
    }
    Some(v as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aac_lc_44100_stereo() {
        // AAC LC (object_type=2), 44100Hz (sf_index=4), 2ch
        // 00010 0100 0010 000 = 0x12 0x10
        let cfg = parse_audio_specific_config(&[0x12, 0x10]).unwrap();
        assert_eq!(cfg.object_type, 2);
        assert_eq!(cfg.sample_rate, 44100);
        assert_eq!(cfg.channels, 2);
    }
}
