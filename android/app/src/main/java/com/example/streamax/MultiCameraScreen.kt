package com.example.streamax

import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView

/**
 * Multi-camera grid. Each cell owns its own StreamaxPlayer + SurfaceView;
 * the cells share no state. Pass a list of (label, url) pairs.
 */
@Composable
fun MultiCameraScreen(cameras: List<Camera>) {
    LazyVerticalGrid(
        columns = GridCells.Fixed(if (cameras.size <= 1) 1 else 2),
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black),
        contentPadding = PaddingValues(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        items(cameras, key = { it.id }) { cam -> CameraTile(cam) }
    }
}

data class Camera(val id: String, val label: String, val url: String)

@Composable
private fun CameraTile(camera: Camera) {
    val player = remember { StreamaxPlayer() }
    var state by remember { mutableStateOf(StreamaxPlayer.State.IDLE) }

    DisposableEffect(camera.id) {
        // Optionally configure pinning at runtime (kept out of the binary):
        //   player.certificatePinner = CertificatePinner.Builder()
        //       .add(host, "sha256/$spki").build()
        player.stateListener = object : StreamaxPlayer.StateListener {
            override fun onState(s: StreamaxPlayer.State) { state = s }
            override fun onError(message: String) {}
        }
        onDispose { player.shutdown() }
    }

    Box(
        modifier = Modifier
            .fillMaxWidth()
            .aspectRatio(16f / 9f)
            .background(Color.Black),
    ) {
        AndroidView(
            factory = { ctx ->
                SurfaceView(ctx).apply {
                    holder.addCallback(object : SurfaceHolder.Callback {
                        override fun surfaceCreated(holder: SurfaceHolder) {
                            player.surface = holder.surface
                            player.play(camera.url)
                        }
                        override fun surfaceChanged(h: SurfaceHolder, f: Int, w: Int, hh: Int) {}
                        override fun surfaceDestroyed(holder: SurfaceHolder) {
                            player.surface = null
                            player.stop()
                        }
                    })
                }
            },
            modifier = Modifier.fillMaxSize(),
        )

        // Label + status dot overlay.
        Row(
            modifier = Modifier
                .align(Alignment.TopStart)
                .padding(6.dp)
                .background(Color.Black.copy(alpha = 0.5f))
                .padding(horizontal = 6.dp, vertical = 3.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            val dot = when (state) {
                StreamaxPlayer.State.PLAYING -> Color(0xFF34D399)
                StreamaxPlayer.State.RECONNECTING, StreamaxPlayer.State.CONNECTING -> Color(0xFFFBBF24)
                StreamaxPlayer.State.ERROR -> Color(0xFFF87171)
                else -> Color.Gray
            }
            Box(Modifier
                .size(8.dp)
                .background(dot, shape = androidx.compose.foundation.shape.CircleShape))
            Spacer(Modifier.width(6.dp))
            Text(
                text = camera.label,
                color = Color.White,
                style = MaterialTheme.typography.labelSmall,
            )
        }
    }
}
