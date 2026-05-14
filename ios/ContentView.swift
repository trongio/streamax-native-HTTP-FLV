import AVFoundation
import SwiftUI

struct ContentView: View {
    @State private var urlString = ""
    @State private var state: StreamaxPlayer.State = .idle
    @State private var errorMessage: String?
    @StateObject private var model = PlayerModel()

    var body: some View {
        VStack(spacing: 12) {
            PlayerView(player: model.player)
                .background(Color.black)
                .aspectRatio(16.0/9.0, contentMode: .fit)
                .cornerRadius(8)

            TextField("Paste a live FLV URL", text: $urlString, axis: .vertical)
                .font(.system(.footnote, design: .monospaced))
                .lineLimit(1...4)
                .textFieldStyle(.roundedBorder)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)

            HStack(spacing: 12) {
                Button(playButtonLabel) {
                    if isActive {
                        model.player.stop()
                    } else if let url = URL(string: urlString.trimmingCharacters(in: .whitespacesAndNewlines)) {
                        model.player.play(url: url)
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(!isActive && urlString.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)

                Spacer()

                Text(statusLabel)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }

            if let err = errorMessage {
                Text(err)
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding()
        .onAppear {
            try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .moviePlayback)
            try? AVAudioSession.sharedInstance().setActive(true)
            model.player.onStateChange = { state = $0 }
            model.player.onError = { errorMessage = $0 }
        }
        .onDisappear { model.player.stop() }
    }

    private var isActive: Bool {
        state == .playing || state == .connecting || state == .reconnecting
    }
    private var playButtonLabel: String { isActive ? "Stop" : "Play" }
    private var statusLabel: String {
        switch state {
        case .playing:      return "Streaming…"
        case .connecting:   return "Connecting…"
        case .reconnecting: return "Reconnecting…"
        case .error:        return "Error"
        case .stopped:      return "Stopped"
        case .idle:         return "Idle"
        }
    }
}

@available(iOS 14.0, *)
final class PlayerModel: ObservableObject {
    let player = StreamaxPlayer()
    // To enable optional SPKI pinning at runtime, set after construction:
    //   model.player.pinnedSPKIHashes = ["<base64-sha256-of-your-server-spki>"]
    // To trust a self-signed cert on a specific host (defense-in-depth alternative):
    //   model.player.trustedInsecureHosts = ["your.camera.host"]
}

#Preview { ContentView() }
