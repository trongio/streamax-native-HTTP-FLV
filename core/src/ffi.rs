//! C ABI for the demuxer. Pull-style — no callbacks.
//!
//! Typical usage from any host language:
//!
//!   d = streamax_demuxer_new()
//!   loop:
//!     bytes = network_read()
//!     streamax_demuxer_append(d, bytes, len)
//!     loop:
//!       ev = streamax_demuxer_next_event(d)
//!       if ev.kind == NONE: break
//!       handle(ev)
//!   streamax_demuxer_free(d)
//!
//! All pointers in the returned [`CEvent`] are owned by the demuxer and
//! invalidated by the next call to `next_event` / `append` / `free`.

use crate::flv::{Demuxer, Event};
use crate::mpegts::Muxer;
use std::os::raw::c_void;

#[repr(C)]
#[derive(Copy, Clone)]
pub enum EventKind {
    None = 0,
    VideoConfig = 1,
    VideoFrame = 2,
    AudioConfig = 3,
    AudioFrame = 4,
    Error = 99,
}

#[repr(C)]
pub struct CEvent {
    pub kind: EventKind,
    pub data: *const u8,
    pub data_len: usize,
    pub pts_ms: u32,
    pub is_keyframe: u8,
    /// width (VideoConfig only)
    pub width: u32,
    /// height (VideoConfig only)
    pub height: u32,
    /// AAC sample rate (AudioConfig only)
    pub sample_rate: u32,
    /// AAC channel count (AudioConfig only)
    pub channels: u8,
    /// AAC object type (AudioConfig only)
    pub object_type: u8,
}

impl Default for CEvent {
    fn default() -> Self {
        Self {
            kind: EventKind::None,
            data: std::ptr::null(),
            data_len: 0,
            pts_ms: 0,
            is_keyframe: 0,
            width: 0,
            height: 0,
            sample_rate: 0,
            channels: 0,
            object_type: 0,
        }
    }
}

/// Demuxer + a "last event" buffer that keeps the bytes alive across
/// FFI calls. Hosts dereference `event.data` until the next call into the
/// demuxer; then the buffer is overwritten.
pub struct Handle {
    demuxer: Demuxer,
    last_buf: Vec<u8>,
    last_err: Option<std::ffi::CString>,
    /// Optional MPEG-TS muxer. When [`streamax_demuxer_set_ts_mode`] enables
    /// it, the host pulls TS bytes via [`streamax_demuxer_next_ts`] instead
    /// of (or in addition to) events.
    ts: Option<Muxer>,
}

#[no_mangle]
pub extern "C" fn streamax_demuxer_new() -> *mut c_void {
    let h = Box::new(Handle {
        demuxer: Demuxer::new(),
        last_buf: Vec::new(),
        last_err: None,
        ts: None,
    });
    Box::into_raw(h) as *mut c_void
}

/// Enable / disable MPEG-TS muxing. When enabled, the host should call
/// [`streamax_demuxer_next_ts`] after each append to drain TS bytes.
/// `enable != 0` turns it on; `0` turns it off and discards any pending mux state.
#[no_mangle]
pub unsafe extern "C" fn streamax_demuxer_set_ts_mode(handle: *mut c_void, enable: u8) {
    if let Some(h) = (handle as *mut Handle).as_mut() {
        h.ts = if enable != 0 { Some(Muxer::new()) } else { None };
    }
}

/// Drains events into the muxer and returns the muxed bytes (if any). The
/// returned pointer is valid until the next call into the demuxer. Returns
/// data_len = 0 when nothing is ready yet (caller should append more bytes).
///
/// Requires [`streamax_demuxer_set_ts_mode`] to have enabled TS mode first.
#[no_mangle]
pub unsafe extern "C" fn streamax_demuxer_next_ts(
    handle: *mut c_void,
    out_data: *mut *const u8,
    out_len: *mut usize,
) -> u8 {
    let h = match (handle as *mut Handle).as_mut() {
        Some(h) => h,
        None => return 0,
    };
    if out_data.is_null() || out_len.is_null() {
        return 0;
    }
    let Some(muxer) = h.ts.as_mut() else {
        return 0;
    };

    while let Some(ev) = h.demuxer.next_event() {
        match ev {
            Event::VideoConfig { annex_b, .. } => muxer.push_video_config(&annex_b),
            Event::VideoFrame { annex_b, pts_ms, is_keyframe } => {
                muxer.push_video_frame(&annex_b, pts_ms, is_keyframe);
            }
            Event::AudioConfig { sample_rate, channels, object_type, .. } => {
                muxer.push_audio_config(sample_rate, channels, object_type);
            }
            Event::AudioFrame { data, pts_ms } => muxer.push_audio_frame(&data, pts_ms),
            Event::Error(msg) => {
                h.last_err = std::ffi::CString::new(msg).ok();
                *out_data = h.last_err.as_ref().map_or(std::ptr::null(), |c| c.as_ptr() as *const u8);
                *out_len = h.last_err.as_ref().map_or(0, |c| c.as_bytes().len());
                return 2; // signals error
            }
        }
    }

    let bytes = muxer.drain();
    if bytes.is_empty() {
        *out_data = std::ptr::null();
        *out_len = 0;
        return 0;
    }
    h.last_buf = bytes;
    *out_data = h.last_buf.as_ptr();
    *out_len = h.last_buf.len();
    1
}

#[no_mangle]
pub unsafe extern "C" fn streamax_demuxer_free(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    let _ = Box::from_raw(handle as *mut Handle);
}

#[no_mangle]
pub unsafe extern "C" fn streamax_demuxer_reset(handle: *mut c_void) {
    if let Some(h) = (handle as *mut Handle).as_mut() {
        h.demuxer.reset();
        h.last_buf.clear();
        h.last_err = None;
        if h.ts.is_some() {
            h.ts = Some(Muxer::new());
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn streamax_demuxer_append(
    handle: *mut c_void,
    data: *const u8,
    len: usize,
) {
    let h = match (handle as *mut Handle).as_mut() {
        Some(h) => h,
        None => return,
    };
    if data.is_null() || len == 0 {
        return;
    }
    let slice = std::slice::from_raw_parts(data, len);
    h.demuxer.append(slice);
}

/// Fills `out` with the next event. Returns 1 if an event is present, 0 if not.
#[no_mangle]
pub unsafe extern "C" fn streamax_demuxer_next_event(
    handle: *mut c_void,
    out: *mut CEvent,
) -> u8 {
    let h = match (handle as *mut Handle).as_mut() {
        Some(h) => h,
        None => return 0,
    };
    if out.is_null() {
        return 0;
    }

    let mut ev_c = CEvent::default();
    let Some(ev) = h.demuxer.next_event() else {
        *out = ev_c;
        return 0;
    };

    match ev {
        Event::VideoConfig {
            annex_b,
            width,
            height,
        } => {
            h.last_buf = annex_b;
            ev_c.kind = EventKind::VideoConfig;
            ev_c.data = h.last_buf.as_ptr();
            ev_c.data_len = h.last_buf.len();
            ev_c.width = width;
            ev_c.height = height;
        }
        Event::VideoFrame {
            annex_b,
            pts_ms,
            is_keyframe,
        } => {
            h.last_buf = annex_b;
            ev_c.kind = EventKind::VideoFrame;
            ev_c.data = h.last_buf.as_ptr();
            ev_c.data_len = h.last_buf.len();
            ev_c.pts_ms = pts_ms;
            ev_c.is_keyframe = if is_keyframe { 1 } else { 0 };
        }
        Event::AudioConfig {
            config,
            sample_rate,
            channels,
            object_type,
        } => {
            h.last_buf = config;
            ev_c.kind = EventKind::AudioConfig;
            ev_c.data = h.last_buf.as_ptr();
            ev_c.data_len = h.last_buf.len();
            ev_c.sample_rate = sample_rate;
            ev_c.channels = channels;
            ev_c.object_type = object_type;
        }
        Event::AudioFrame { data, pts_ms } => {
            h.last_buf = data;
            ev_c.kind = EventKind::AudioFrame;
            ev_c.data = h.last_buf.as_ptr();
            ev_c.data_len = h.last_buf.len();
            ev_c.pts_ms = pts_ms;
        }
        Event::Error(msg) => {
            h.last_err = std::ffi::CString::new(msg).ok();
            ev_c.kind = EventKind::Error;
            if let Some(c) = h.last_err.as_ref() {
                ev_c.data = c.as_ptr() as *const u8;
                ev_c.data_len = c.as_bytes().len();
            }
        }
    }

    *out = ev_c;
    1
}
