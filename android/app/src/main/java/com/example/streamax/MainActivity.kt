package com.example.streamax

import android.app.PictureInPictureParams
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import android.util.Rational
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.foundation.text.KeyboardOptions

class MainActivity : ComponentActivity() {

    // Latest video size, surfaced via StreamaxPlayer.StateListener.onVideoSize.
    @Volatile private var videoAspect: Rational = Rational(16, 9)
    val inPip: MutableState<Boolean> = mutableStateOf(false)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { MaterialTheme { PlayerScreen(activity = this) } }
    }

    fun updateVideoAspect(w: Int, h: Int) {
        if (w > 0 && h > 0) videoAspect = Rational(w, h)
    }

    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        // Auto-enter PiP when the user presses Home while streaming.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val params = PictureInPictureParams.Builder()
                .setAspectRatio(videoAspect)
                .build()
            try { enterPictureInPictureMode(params) } catch (_: Exception) {}
        }
    }

    override fun onPictureInPictureModeChanged(isInPictureInPictureMode: Boolean, newConfig: Configuration) {
        super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
        inPip.value = isInPictureInPictureMode
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PlayerScreen(activity: MainActivity? = null) {
    val player = remember { StreamaxPlayer() }
    var url by remember {
        mutableStateOf("https://YOUR-CAMERA-HOST.example.com:22060/live.flv?devid=YOUR-DEVID&chl=1&st=1&audio=0&hash=anything")
    }
    var state by remember { mutableStateOf(StreamaxPlayer.State.IDLE) }
    var error by remember { mutableStateOf<String?>(null) }
    val inPip = activity?.inPip?.value ?: false

    DisposableEffect(Unit) {
        // SHA-256 of the SubjectPublicKeyInfo for YOUR-CAMERA-HOST.example.com:22060
        // (Sectigo-issued cert, expires 2026-12-28). Re-pin if the cert key rotates.
        player.certificatePinner = okhttp3.CertificatePinner.Builder()
            .add("YOUR-CAMERA-HOST.example.com", "sha256/YOUR-SPKI-HASH-BASE64=")
            .build()
        player.stateListener = object : StreamaxPlayer.StateListener {
            override fun onState(s: StreamaxPlayer.State) { state = s }
            override fun onError(message: String) { error = message }
            override fun onVideoSize(width: Int, height: Int) {
                activity?.updateVideoAspect(width, height)
            }
        }
        onDispose {
            player.stateListener = null
            player.shutdown()
        }
    }

    Column(modifier = Modifier
        .fillMaxSize()
        .padding(if (inPip) 0.dp else 16.dp)) {

        AndroidView(
            factory = { ctx ->
                SurfaceView(ctx).apply {
                    holder.addCallback(object : SurfaceHolder.Callback {
                        override fun surfaceCreated(holder: SurfaceHolder) {
                            player.surface = holder.surface
                        }
                        override fun surfaceChanged(h: SurfaceHolder, f: Int, w: Int, hh: Int) {}
                        override fun surfaceDestroyed(holder: SurfaceHolder) {
                            player.surface = null
                            player.stop()
                        }
                    })
                }
            },
            modifier = Modifier
                .fillMaxWidth()
                .then(if (inPip) Modifier.weight(1f) else Modifier.aspectRatio(16f / 9f))
                .background(Color.Black)
        )

        // PiP hides the URL field and buttons — render only the video.
        if (inPip) return@Column

        Spacer(Modifier.height(12.dp))

        OutlinedTextField(
            value = url,
            onValueChange = { url = it; error = null },
            modifier = Modifier.fillMaxWidth(),
            label = { Text("Live FLV URL") },
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Uri,
                capitalization = KeyboardCapitalization.None
            ),
            singleLine = false,
            maxLines = 4,
        )

        Spacer(Modifier.height(12.dp))

        Row(verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            val playing = state == StreamaxPlayer.State.PLAYING ||
                          state == StreamaxPlayer.State.CONNECTING
            Button(onClick = {
                if (playing) player.stop() else player.play(url.trim())
            }) { Text(if (playing) "Stop" else "Play") }

            Spacer(Modifier.width(12.dp))
            Text(when (state) {
                StreamaxPlayer.State.PLAYING -> "Streaming…"
                StreamaxPlayer.State.CONNECTING -> "Connecting…"
                StreamaxPlayer.State.RECONNECTING -> "Reconnecting…"
                StreamaxPlayer.State.ERROR -> "Error"
                StreamaxPlayer.State.STOPPED -> "Stopped"
                else -> "Idle"
            }, style = MaterialTheme.typography.bodySmall)
        }

        error?.let {
            Spacer(Modifier.height(8.dp))
            Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
        }
    }
}
