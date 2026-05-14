import AVFoundation
import SwiftUI

/// 2-, 3- or 4-up grid of live streams. Each tile owns its own
/// StreamaxPlayer + display layer; tiles share no state.
@available(iOS 14.0, *)
struct MultiCameraView: View {

    struct Camera: Identifiable {
        let id = UUID()
        let label: String
        let url: URL
    }

    let cameras: [Camera]

    private var columns: [GridItem] {
        let n = cameras.count <= 1 ? 1 : 2
        return Array(repeating: GridItem(.flexible(), spacing: 8), count: n)
    }

    var body: some View {
        ScrollView {
            LazyVGrid(columns: columns, spacing: 8) {
                ForEach(cameras) { cam in
                    CameraTile(camera: cam)
                        .aspectRatio(16.0/9.0, contentMode: .fit)
                        .background(Color.black)
                        .cornerRadius(6)
                }
            }
            .padding(8)
        }
        .background(Color.black.ignoresSafeArea())
        .onAppear {
            try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .moviePlayback)
            try? AVAudioSession.sharedInstance().setActive(true)
        }
    }
}

@available(iOS 14.0, *)
private struct CameraTile: View {
    let camera: MultiCameraView.Camera
    @StateObject private var model = CameraTileModel()
    @State private var failed = false

    var body: some View {
        ZStack(alignment: .topLeading) {
            PlayerView(player: model.player)

            // Label overlay
            HStack {
                Circle()
                    .fill(failed ? Color.red : Color.green)
                    .frame(width: 8, height: 8)
                Text(camera.label)
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(.white)
            }
            .padding(6)
            .background(.black.opacity(0.5))
            .cornerRadius(4)
            .padding(6)
        }
        .onAppear {
            model.player.onError = { _ in failed = true }
            model.player.onStateChange = { state in
                if state == .playing { failed = false }
            }
            model.player.play(url: camera.url)
        }
        .onDisappear { model.player.stop() }
    }
}

@available(iOS 14.0, *)
private final class CameraTileModel: ObservableObject {
    let player = StreamaxPlayer()
    // To enable SPKI pinning, set player.pinnedSPKIHashes after construction.
}

#Preview {
    if #available(iOS 14.0, *) {
        // Preview cameras: callers pass their own list of URLs at runtime.
        MultiCameraView(cameras: [])
    }
}
