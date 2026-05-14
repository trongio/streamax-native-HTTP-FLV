package com.example.streamax

import android.os.Bundle
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

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { MaterialTheme { PlayerScreen() } }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PlayerScreen() {
    val player = remember { StreamaxPlayer() }
    var url by remember {
        mutableStateOf("https://YOUR-CAMERA-HOST.example.com:22060/live.flv?devid=YOUR-DEVID&chl=1&st=1&audio=0&hash=anything")
    }
    var state by remember { mutableStateOf(StreamaxPlayer.State.IDLE) }
    var error by remember { mutableStateOf<String?>(null) }

    DisposableEffect(Unit) {
        player.stateListener = object : StreamaxPlayer.StateListener {
            override fun onState(s: StreamaxPlayer.State) { state = s }
            override fun onError(message: String) { error = message }
        }
        onDispose {
            player.stateListener = null
            player.shutdown()
        }
    }

    Column(modifier = Modifier
        .fillMaxSize()
        .padding(16.dp)) {

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
                .aspectRatio(16f / 9f)
                .background(Color.Black)
        )

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
