package com.example.streamax

/**
 * Kotlin wrapper for the Rust streamax-core native library.
 *
 * Build pipeline:
 *   1. Build `streamax-core` for Android (see streamax-core/build.sh).
 *   2. Copy libstreamax_core.so + the JNI bridge into `src/main/jniLibs/<abi>/`.
 *
 * The JNI bridge is in `app/src/main/cpp/streamax_jni.c`; it links against
 * libstreamax_core.so and exposes the methods declared here.
 *
 * Single-threaded — create one instance per stream.
 */
class StreamaxCore : AutoCloseable {

    sealed class Event {
        data class VideoConfig(val annexB: ByteArray, val width: Int, val height: Int) : Event()
        data class VideoFrame(val annexB: ByteArray, val ptsMs: Int, val isKeyframe: Boolean) : Event()
        data class AudioConfig(
            val config: ByteArray,
            val sampleRate: Int,
            val channels: Int,
            val objectType: Int,
        ) : Event()
        data class AudioFrame(val data: ByteArray, val ptsMs: Int) : Event()
        data class Error(val message: String) : Event()
    }

    private var handle: Long = nativeNew()

    fun reset() {
        if (handle != 0L) nativeReset(handle)
    }

    fun append(chunk: ByteArray, length: Int) {
        if (handle != 0L && length > 0) nativeAppend(handle, chunk, length)
    }

    fun nextEvent(): Event? {
        if (handle == 0L) return null
        val ev = nativeNextEvent(handle) ?: return null
        return when (ev.kind) {
            KIND_VIDEO_CONFIG -> Event.VideoConfig(ev.data, ev.width, ev.height)
            KIND_VIDEO_FRAME -> Event.VideoFrame(ev.data, ev.ptsMs, ev.isKeyframe)
            KIND_AUDIO_CONFIG -> Event.AudioConfig(ev.data, ev.sampleRate, ev.channels, ev.objectType)
            KIND_AUDIO_FRAME -> Event.AudioFrame(ev.data, ev.ptsMs)
            KIND_ERROR -> Event.Error(String(ev.data, Charsets.UTF_8))
            else -> null
        }
    }

    override fun close() {
        if (handle != 0L) {
            nativeFree(handle)
            handle = 0L
        }
    }

    private external fun nativeNew(): Long
    private external fun nativeFree(handle: Long)
    private external fun nativeReset(handle: Long)
    private external fun nativeAppend(handle: Long, data: ByteArray, length: Int)
    private external fun nativeNextEvent(handle: Long): NativeEvent?

    private class NativeEvent {
        @JvmField var kind: Int = 0
        @JvmField var data: ByteArray = ByteArray(0)
        @JvmField var ptsMs: Int = 0
        @JvmField var isKeyframe: Boolean = false
        @JvmField var width: Int = 0
        @JvmField var height: Int = 0
        @JvmField var sampleRate: Int = 0
        @JvmField var channels: Int = 0
        @JvmField var objectType: Int = 0
    }

    companion object {
        private const val KIND_VIDEO_CONFIG = 1
        private const val KIND_VIDEO_FRAME = 2
        private const val KIND_AUDIO_CONFIG = 3
        private const val KIND_AUDIO_FRAME = 4
        private const val KIND_ERROR = 99

        init { System.loadLibrary("streamax_jni") }
    }
}
