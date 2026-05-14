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
}

#[no_mangle]
pub extern "C" fn streamax_demuxer_new() -> *mut c_void {
    let h = Box::new(Handle {
        demuxer: Demuxer::new(),
        last_buf: Vec::new(),
        last_err: None,
    });
    Box::into_raw(h) as *mut c_void
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
