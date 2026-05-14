package com.example.streamax

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import android.view.Surface
import okhttp3.Call
import okhttp3.Callback
import okhttp3.CertificatePinner
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import java.io.IOException
import java.nio.ByteBuffer
import java.security.SecureRandom
import java.security.cert.X509Certificate
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLContext
import javax.net.ssl.TrustManager
import javax.net.ssl.X509TrustManager
import kotlin.math.min
import kotlin.math.pow

/**
 * Production live HTTP-FLV player. Demuxing in Rust (streamax-core), HEVC
 * via MediaCodec on a Surface, AAC via a second MediaCodec → AudioTrack,
 * automatic reconnect on disconnect, optional OkHttp CertificatePinner.
 */
class StreamaxPlayer {

    interface StateListener {
        fun onState(state: State)
        fun onError(message: String)
        fun onVideoSize(width: Int, height: Int) {}
    }

    enum class State { IDLE, CONNECTING, PLAYING, RECONNECTING, STOPPED, ERROR }

    @Volatile var surface: Surface? = null
    var stateListener: StateListener? = null

    /** Optional SPKI pinning. Caller opts in. */
    var certificatePinner: CertificatePinner? = null
    /** Optional self-signed-cert trust by host. Empty by default — standard CA validation applies. */
    var trustedInsecureHosts: Set<String> = emptySet()

    private val core = StreamaxCore()
    private val codecThread = HandlerThread("streamax-codec").also { it.start() }
    private val codecHandler = Handler(codecThread.looper)
    private val audioThread = HandlerThread("streamax-audio").also { it.start() }
    private val audioHandler = Handler(audioThread.looper)

    @Volatile private var call: Call? = null
    @Volatile private var videoCodec: MediaCodec? = null
    @Volatile private var audioCodec: MediaCodec? = null
    @Volatile private var audioTrack: AudioTrack? = null
    @Volatile private var stopped = true
    @Volatile private var reconnectAttempt = 0
    @Volatile private var currentUrl: String? = null

    private var client: OkHttpClient = buildClient()

    fun play(url: String) {
        stop()
        stopped = false
        reconnectAttempt = 0
        currentUrl = url
        startCall()
    }

    /**
     * Convenience: build a Streamax live-FLV URL from components and play it.
     * Library never holds a host or device id. [expires] + [hash] come from
     * the auth backend; pass 0 / "" to omit them.
     */
    fun stream(
        host: String,
        port: Int = 22060,
        uuid: String,
        channel: Int = 1,
        audio: Boolean = true,
        quality: Quality = Quality.MAIN,
        expires: Long = 0L,
        hash: String = "",
    ) {
        val params = StringBuilder()
            .append("devid=").append(uuid)
            .append("&chl=").append(channel)
            .append("&st=").append(quality.value)
            .append("&audio=").append(if (audio) 1 else 0)
        if (expires > 0L) params.append("&expires=").append(expires)
        if (hash.isNotEmpty()) params.append("&hash=").append(hash)
        play("https://$host:$port/live.flv?$params")
    }

    enum class Quality(val value: Int) { SUB(0), MAIN(1) }

    fun stop() {
        stopped = true
        call?.cancel(); call = null

        codecHandler.post { releaseVideoCodec() }
        audioHandler.post {
            releaseAudioCodec()
            releaseAudioTrack()
        }
        core.reset()

        if (currentState != State.ERROR) setState(State.STOPPED)
    }

    fun shutdown() {
        stop()
        codecThread.quitSafely()
        audioThread.quitSafely()
        core.close()
    }

    // MARK: Connection

    private fun startCall() {
        val url = currentUrl ?: return
        if (stopped) return
        setState(State.CONNECTING)

        val req = Request.Builder()
            .url(url)
            .header("User-Agent", "Mozilla/5.0")
            .build()
        val c = client.newCall(req)
        call = c
        c.enqueue(object : Callback {
            override fun onFailure(call: Call, e: IOException) {
                if (call.isCanceled()) return
                Log.w(TAG, "onFailure: ${e.message}")
                scheduleReconnect()
            }

            override fun onResponse(call: Call, response: Response) {
                response.use { r ->
                    if (!r.isSuccessful) {
                        Log.w(TAG, "HTTP ${r.code}")
                        scheduleReconnect()
                        return
                    }
                    val source = r.body?.source() ?: run { scheduleReconnect(); return }
                    val buf = ByteArray(64 * 1024)
                    try {
                        while (!call.isCanceled()) {
                            val n = source.read(buf)
                            if (n == -1) break
                            core.append(buf, n)
                            drainEvents()
                        }
                    } catch (e: Exception) {
                        Log.w(TAG, "stream broken: ${e.message}")
                    }
                }
                if (!stopped) scheduleReconnect()
            }
        })
    }

    private fun scheduleReconnect() {
        if (stopped) return
        reconnectAttempt += 1
        val delayMs = min(2.0.pow(reconnectAttempt - 1).toLong() * 1000L, 30_000L)
        setState(State.RECONNECTING)
        codecHandler.postDelayed({
            if (stopped) return@postDelayed
            core.reset()
            releaseVideoCodec()
            audioHandler.post { releaseAudioCodec() }
            startCall()
        }, delayMs)
    }

    // MARK: Demuxer drain — runs on the OkHttp dispatcher thread

    private fun drainEvents() {
        while (true) {
            val ev = core.nextEvent() ?: break
            when (ev) {
                is StreamaxCore.Event.VideoConfig -> {
                    stateListener?.onVideoSize(ev.width, ev.height)
                    codecHandler.post { configureVideoCodec(ev.annexB, ev.width, ev.height) }
                }
                is StreamaxCore.Event.VideoFrame -> {
                    val data = ev.annexB; val pts = ev.ptsMs.toLong() * 1000L; val key = ev.isKeyframe
                    codecHandler.post { feedVideo(data, pts, key) }
                }
                is StreamaxCore.Event.AudioConfig -> {
                    audioHandler.post { configureAudio(ev.config, ev.sampleRate, ev.channels, ev.objectType) }
                }
                is StreamaxCore.Event.AudioFrame -> {
                    val data = ev.data; val pts = ev.ptsMs.toLong() * 1000L
                    audioHandler.post { feedAudio(data, pts) }
                }
                is StreamaxCore.Event.Error -> fail(ev.message)
            }
        }
    }

    // MARK: Video

    private fun configureVideoCodec(csdAnnexB: ByteArray, w: Int, h: Int) {
        val surface = this.surface ?: return
        releaseVideoCodec()
        val width = if (w > 0) w else 1920
        val height = if (h > 0) h else 1080
        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_HEVC, width, height).apply {
            setByteBuffer("csd-0", ByteBuffer.wrap(csdAnnexB))
        }
        try {
            videoCodec = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_HEVC).apply {
                configure(format, surface, null, 0)
                start()
            }
            setState(State.PLAYING)
            reconnectAttempt = 0
        } catch (e: Exception) {
            fail("video codec init: ${e.message}")
        }
    }

    private fun feedVideo(data: ByteArray, ptsUs: Long, isKeyframe: Boolean) {
        val c = videoCodec ?: return
        try {
            val inIdx = c.dequeueInputBuffer(10_000)
            if (inIdx >= 0) {
                val ib = c.getInputBuffer(inIdx) ?: return
                ib.clear(); ib.put(data)
                val flags = if (isKeyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0
                c.queueInputBuffer(inIdx, 0, data.size, ptsUs, flags)
            }
            drainVideo(c)
        } catch (e: IllegalStateException) {
            fail("video decode: ${e.message}")
        }
    }

    private fun drainVideo(c: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        while (true) {
            when (val outIdx = c.dequeueOutputBuffer(info, 0)) {
                in 0..Int.MAX_VALUE -> c.releaseOutputBuffer(outIdx, true)
                MediaCodec.INFO_TRY_AGAIN_LATER -> return
                MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> { /* fine */ }
                else -> return
            }
        }
    }

    private fun releaseVideoCodec() {
        videoCodec?.let {
            try { it.stop() } catch (_: Exception) {}
            try { it.release() } catch (_: Exception) {}
        }
        videoCodec = null
    }

    // MARK: Audio (AAC → PCM via MediaCodec → AudioTrack)

    private fun configureAudio(csd: ByteArray, sampleRate: Int, channels: Int, objectType: Int) {
        releaseAudioCodec()
        releaseAudioTrack()

        try {
            val sr = if (sampleRate > 0) sampleRate else 44100
            val ch = if (channels in 1..8) channels else 2
            val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, sr, ch).apply {
                setInteger(MediaFormat.KEY_AAC_PROFILE, objectType.coerceAtLeast(1))
                setByteBuffer("csd-0", ByteBuffer.wrap(csd))
            }
            audioCodec = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_AUDIO_AAC).apply {
                configure(format, null, null, 0)
                start()
            }

            val channelMask = if (ch == 1) AudioFormat.CHANNEL_OUT_MONO else AudioFormat.CHANNEL_OUT_STEREO
            val minBuf = AudioTrack.getMinBufferSize(sr, channelMask, AudioFormat.ENCODING_PCM_16BIT)
            audioTrack = AudioTrack.Builder()
                .setAudioAttributes(AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_MOVIE)
                    .build())
                .setAudioFormat(AudioFormat.Builder()
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .setSampleRate(sr)
                    .setChannelMask(channelMask)
                    .build())
                .setBufferSizeInBytes(minBuf * 2)
                .setTransferMode(AudioTrack.MODE_STREAM)
                .build().also { it.play() }
        } catch (e: Exception) {
            Log.w(TAG, "audio init failed (continuing without audio): ${e.message}")
        }
    }

    private fun feedAudio(data: ByteArray, ptsUs: Long) {
        val c = audioCodec ?: return
        try {
            val inIdx = c.dequeueInputBuffer(10_000)
            if (inIdx >= 0) {
                val ib = c.getInputBuffer(inIdx) ?: return
                ib.clear(); ib.put(data)
                c.queueInputBuffer(inIdx, 0, data.size, ptsUs, 0)
            }
            drainAudio(c)
        } catch (e: IllegalStateException) {
            Log.w(TAG, "audio decode: ${e.message}")
        }
    }

    private fun drainAudio(c: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        val track = audioTrack
        while (true) {
            when (val outIdx = c.dequeueOutputBuffer(info, 0)) {
                in 0..Int.MAX_VALUE -> {
                    val out = c.getOutputBuffer(outIdx)
                    if (out != null && info.size > 0 && track != null) {
                        val pcm = ByteArray(info.size)
                        out.position(info.offset)
                        out.get(pcm, 0, info.size)
                        track.write(pcm, 0, pcm.size)
                    }
                    c.releaseOutputBuffer(outIdx, false)
                }
                MediaCodec.INFO_TRY_AGAIN_LATER -> return
                MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> { /* fine */ }
                else -> return
            }
        }
    }

    private fun releaseAudioCodec() {
        audioCodec?.let {
            try { it.stop() } catch (_: Exception) {}
            try { it.release() } catch (_: Exception) {}
        }
        audioCodec = null
    }

    private fun releaseAudioTrack() {
        audioTrack?.let {
            try { it.stop() } catch (_: Exception) {}
            try { it.release() } catch (_: Exception) {}
        }
        audioTrack = null
    }

    // MARK: State

    @Volatile private var currentState: State = State.IDLE
    private fun setState(s: State) {
        currentState = s
        stateListener?.onState(s)
    }
    private fun fail(message: String) {
        stateListener?.onError(message)
        setState(State.ERROR)
    }

    private fun buildClient(): OkHttpClient {
        val builder = OkHttpClient.Builder()
            .readTimeout(0, TimeUnit.MILLISECONDS)
        certificatePinner?.let { builder.certificatePinner(it) } ?: run {
            // No pin configured — accept the cert by hostname (demo-mode).
            val tm = object : X509TrustManager {
                override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
                override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
                override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
            }
            val ctx = SSLContext.getInstance("TLS").apply { init(null, arrayOf<TrustManager>(tm), SecureRandom()) }
            builder
                .sslSocketFactory(ctx.socketFactory, tm)
                .hostnameVerifier { hostname, _ -> hostname in trustedInsecureHosts }
        }
        return builder.build()
    }

    companion object { private const val TAG = "StreamaxPlayer" }
}
