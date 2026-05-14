import Foundation

/// Swift-flavored wrapper around the Rust streamax-core C ABI.
/// Single-threaded — call from one queue at a time.
final class StreamaxCore {
    enum Event {
        case videoConfig(annexB: Data, width: Int, height: Int)
        case videoFrame(annexB: Data, ptsMs: UInt32, isKeyframe: Bool)
        case audioConfig(config: Data, sampleRate: Int, channels: Int, objectType: Int)
        case audioFrame(data: Data, ptsMs: UInt32)
        case error(String)
    }

    private var handle: OpaquePointer?

    init() {
        handle = OpaquePointer(streamax_demuxer_new())
    }

    deinit {
        if let h = handle { streamax_demuxer_free(UnsafeMutableRawPointer(h)) }
    }

    func reset() {
        guard let h = handle else { return }
        streamax_demuxer_reset(UnsafeMutableRawPointer(h))
    }

    func append(_ chunk: Data) {
        guard let h = handle, !chunk.isEmpty else { return }
        chunk.withUnsafeBytes { raw in
            if let base = raw.bindMemory(to: UInt8.self).baseAddress {
                streamax_demuxer_append(UnsafeMutableRawPointer(h), base, chunk.count)
            }
        }
    }

    func nextEvent() -> Event? {
        guard let h = handle else { return nil }
        var ev = StreamaxEvent()
        let got = streamax_demuxer_next_event(UnsafeMutableRawPointer(h), &ev)
        if got == 0 { return nil }

        // Pointer is valid until the next FFI call — copy out now.
        let data: Data = ev.data_len > 0 && ev.data != nil
            ? Data(bytes: ev.data!, count: ev.data_len)
            : Data()

        switch ev.kind {
        case STREAMAX_EVENT_VIDEO_CONFIG:
            return .videoConfig(annexB: data, width: Int(ev.width), height: Int(ev.height))
        case STREAMAX_EVENT_VIDEO_FRAME:
            return .videoFrame(annexB: data, ptsMs: ev.pts_ms, isKeyframe: ev.is_keyframe == 1)
        case STREAMAX_EVENT_AUDIO_CONFIG:
            return .audioConfig(config: data, sampleRate: Int(ev.sample_rate),
                                channels: Int(ev.channels), objectType: Int(ev.object_type))
        case STREAMAX_EVENT_AUDIO_FRAME:
            return .audioFrame(data: data, ptsMs: ev.pts_ms)
        case STREAMAX_EVENT_ERROR:
            return .error(String(data: data, encoding: .utf8) ?? "demuxer error")
        default:
            return nil
        }
    }
}
