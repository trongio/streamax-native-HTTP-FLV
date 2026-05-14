//! Minimal HEVC SPS parser — extracts just the picture dimensions.
//!
//! Spec reference: ITU-T H.265 (V7) §7.3.2.2.1, §7.4.3.2.1.

/// Returns (width, height) parsed from an HEVC SPS NALU (Annex-B body
/// without start code, NAL header included).
pub fn sps_dimensions(nalu: &[u8]) -> Option<(u32, u32)> {
    if nalu.len() < 3 {
        return None;
    }
    // Strip NAL unit header (2 bytes) and emulation prevention bytes.
    let rbsp = ebsp_to_rbsp(&nalu[2..]);
    let mut r = BitReader::new(&rbsp);

    // sps_video_parameter_set_id (4) + sps_max_sub_layers_minus1 (3) + sps_temporal_id_nesting_flag (1)
    r.read(4)?;
    let max_sub_layers_minus1 = r.read(3)?;
    r.read(1)?;

    profile_tier_level(&mut r, max_sub_layers_minus1)?;

    r.ue()?; // sps_seq_parameter_set_id
    let chroma_format_idc = r.ue()?;
    if chroma_format_idc == 3 {
        r.read(1)?; // separate_colour_plane_flag
    }
    let pic_width_in_luma_samples = r.ue()?;
    let pic_height_in_luma_samples = r.ue()?;

    let conformance_window_flag = r.read(1)?;
    let (mut crop_left, mut crop_right, mut crop_top, mut crop_bottom) = (0u32, 0u32, 0u32, 0u32);
    if conformance_window_flag == 1 {
        crop_left = r.ue()?;
        crop_right = r.ue()?;
        crop_top = r.ue()?;
        crop_bottom = r.ue()?;
    }
    // SubWidthC / SubHeightC (Table 6-1 in the spec, depends on chroma_format_idc)
    let (sub_width_c, sub_height_c) = match chroma_format_idc {
        1 => (2u32, 2u32),
        2 => (2, 1),
        3 => (1, 1),
        _ => (1, 1),
    };

    let width = pic_width_in_luma_samples - sub_width_c * (crop_left + crop_right);
    let height = pic_height_in_luma_samples - sub_height_c * (crop_top + crop_bottom);
    Some((width, height))
}

fn profile_tier_level(r: &mut BitReader, max_sub_layers_minus1: u32) -> Option<()> {
    r.skip(8)?;   // profile_space(2) + tier_flag(1) + profile_idc(5)
    r.skip(32)?;  // general_profile_compatibility_flag[32]
    r.skip(4)?;   // progressive + interlaced + non_packed + frame_only
    r.skip(43)?;  // general_constraint_indicator_flags (or reserved_zero_43bits)
    r.skip(1)?;   // general_inbld_flag or reserved
    r.skip(8)?;   // general_level_idc

    let mut sub_layer_profile_present = Vec::with_capacity(max_sub_layers_minus1 as usize);
    let mut sub_layer_level_present = Vec::with_capacity(max_sub_layers_minus1 as usize);
    for _ in 0..max_sub_layers_minus1 {
        sub_layer_profile_present.push(r.read(1)?);
        sub_layer_level_present.push(r.read(1)?);
    }
    if max_sub_layers_minus1 > 0 {
        for _ in max_sub_layers_minus1..8 {
            r.skip(2)?; // reserved_zero_2bits
        }
    }
    for i in 0..max_sub_layers_minus1 as usize {
        if sub_layer_profile_present[i] == 1 {
            r.skip(8)?;
            r.skip(32)?;
            r.skip(4)?;
            r.skip(43)?;
            r.skip(1)?;
        }
        if sub_layer_level_present[i] == 1 {
            r.skip(8)?;
        }
    }
    Some(())
}

/// Strip 0x000003 emulation-prevention bytes (RBSP).
fn ebsp_to_rbsp(ebsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(ebsp.len());
    let mut zeroes = 0;
    for &b in ebsp {
        if zeroes >= 2 && b == 0x03 {
            zeroes = 0;
            continue;
        }
        if b == 0 {
            zeroes += 1;
        } else {
            zeroes = 0;
        }
        out.push(b);
    }
    out
}

struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn read(&mut self, n: usize) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if n > 32 {
            return None;
        }
        let mut v = 0u64;
        for _ in 0..n {
            let byte = *self.data.get(self.bit_pos / 8)?;
            let bit = (byte >> (7 - (self.bit_pos % 8))) & 1;
            v = (v << 1) | (bit as u64);
            self.bit_pos += 1;
        }
        Some(v as u32)
    }

    /// Advance the cursor n bits without producing a value. Allows n > 32.
    fn skip(&mut self, n: usize) -> Option<()> {
        let new_pos = self.bit_pos + n;
        if new_pos > self.data.len() * 8 {
            return None;
        }
        self.bit_pos = new_pos;
        Some(())
    }

    /// Unsigned exponential-Golomb code.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 32 {
                return None;
            }
        }
        if zeros == 0 {
            return Some(0);
        }
        let rest = self.read(zeros)?;
        Some((1u32 << zeros) - 1 + rest)
    }
}
