/*
 * streamax_core.h — C ABI for streamax-core.
 *
 * Pull API: hand bytes in via _append, then drain events via _next_event
 * until it returns 0. Pointers in StreamaxEvent are owned by the demuxer
 * and invalidated by the next call into the demuxer.
 *
 * Thread safety: each demuxer instance is single-threaded. Caller must
 * serialize calls or use one demuxer per thread.
 */
#ifndef STREAMAX_CORE_H
#define STREAMAX_CORE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    STREAMAX_EVENT_NONE         = 0,
    STREAMAX_EVENT_VIDEO_CONFIG = 1,
    STREAMAX_EVENT_VIDEO_FRAME  = 2,
    STREAMAX_EVENT_AUDIO_CONFIG = 3,
    STREAMAX_EVENT_AUDIO_FRAME  = 4,
    STREAMAX_EVENT_ERROR        = 99,
} StreamaxEventKind;

typedef struct {
    StreamaxEventKind kind;
    const uint8_t*    data;
    size_t            data_len;
    uint32_t          pts_ms;
    uint8_t           is_keyframe;
    uint32_t          width;        /* VIDEO_CONFIG */
    uint32_t          height;       /* VIDEO_CONFIG */
    uint32_t          sample_rate;  /* AUDIO_CONFIG */
    uint8_t           channels;     /* AUDIO_CONFIG */
    uint8_t           object_type;  /* AUDIO_CONFIG */
} StreamaxEvent;

typedef void StreamaxDemuxer;

StreamaxDemuxer* streamax_demuxer_new(void);
void             streamax_demuxer_free(StreamaxDemuxer* d);
void             streamax_demuxer_reset(StreamaxDemuxer* d);
void             streamax_demuxer_append(StreamaxDemuxer* d, const uint8_t* data, size_t len);

/* Returns 1 if an event was written into *out, 0 if no event is pending. */
uint8_t          streamax_demuxer_next_event(StreamaxDemuxer* d, StreamaxEvent* out);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* STREAMAX_CORE_H */
