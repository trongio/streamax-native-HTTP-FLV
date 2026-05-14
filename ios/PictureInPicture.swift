import AVFoundation
import AVKit
import Foundation

/// Picture-in-Picture controller backed by an AVSampleBufferDisplayLayer.
/// Wraps the iOS 15+ API; older iOS gets a no-op.
@available(iOS 15.0, *)
final class PiPController: NSObject {

    private(set) var controller: AVPictureInPictureController?
    var onStateChange: ((Bool) -> Void)?

    init(player: StreamaxPlayer) {
        super.init()
        guard AVPictureInPictureController.isPictureInPictureSupported() else {
            return
        }
        let source = AVPictureInPictureController.ContentSource(
            sampleBufferDisplayLayer: player.displayLayer,
            playbackDelegate: PlaybackDelegate(player: player)
        )
        let c = AVPictureInPictureController(contentSource: source)
        c.canStartPictureInPictureAutomaticallyFromInline = true
        c.delegate = self
        controller = c
    }

    func toggle() {
        guard let c = controller else { return }
        if c.isPictureInPictureActive {
            c.stopPictureInPicture()
        } else if c.isPictureInPicturePossible {
            c.startPictureInPicture()
        }
    }

    private final class PlaybackDelegate: NSObject, AVPictureInPictureSampleBufferPlaybackDelegate {
        weak var player: StreamaxPlayer?
        init(player: StreamaxPlayer) { self.player = player }

        func pictureInPictureController(_ pip: AVPictureInPictureController,
                                        setPlaying playing: Bool) {
            // Live stream — always play.
            DispatchQueue.main.async {
                self.player?.synchronizer.setRate(playing ? 1.0 : 0.0,
                                                  time: self.player?.synchronizer.currentTime() ?? .zero)
            }
        }

        func pictureInPictureControllerTimeRangeForPlayback(_ pip: AVPictureInPictureController) -> CMTimeRange {
            // Indefinite live stream.
            CMTimeRange(start: .negativeInfinity, duration: .positiveInfinity)
        }

        func pictureInPictureControllerIsPlaybackPaused(_ pip: AVPictureInPictureController) -> Bool {
            (player?.synchronizer.rate ?? 0) == 0
        }

        func pictureInPictureController(_ pip: AVPictureInPictureController,
                                        didTransitionToRenderSize newRenderSize: CMVideoDimensions) {
            // Optional: swap to a higher-quality stream if newRenderSize is big.
        }

        func pictureInPictureController(_ pip: AVPictureInPictureController,
                                        skipByInterval skipInterval: CMTime,
                                        completion completionHandler: @escaping () -> Void) {
            // No seeking on a live stream.
            completionHandler()
        }
    }
}

@available(iOS 15.0, *)
extension PiPController: AVPictureInPictureControllerDelegate {
    func pictureInPictureControllerDidStartPictureInPicture(_ c: AVPictureInPictureController) {
        onStateChange?(true)
    }
    func pictureInPictureControllerDidStopPictureInPicture(_ c: AVPictureInPictureController) {
        onStateChange?(false)
    }
}
