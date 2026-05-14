import AVFoundation
import SwiftUI
import UIKit

/// UIView whose backing layer is the AVSampleBufferDisplayLayer.
/// Cheaper than adding a sublayer because resizing/animation route through UIKit.
final class SampleBufferHostView: UIView {
    override class var layerClass: AnyClass { AVSampleBufferDisplayLayer.self }
    var displayLayer: AVSampleBufferDisplayLayer { layer as! AVSampleBufferDisplayLayer }
}

/// SwiftUI wrapper. The player owns its own AVSampleBufferDisplayLayer; we add
/// it as a sublayer so the host view's bounds drive its frame.
struct PlayerView: UIViewRepresentable {
    let player: StreamaxPlayer

    func makeUIView(context: Context) -> UIView {
        let host = UIView()
        host.backgroundColor = .black
        host.layer.addSublayer(player.displayLayer)
        return host
    }

    func updateUIView(_ uiView: UIView, context: Context) {
        player.displayLayer.frame = uiView.bounds
    }
}
