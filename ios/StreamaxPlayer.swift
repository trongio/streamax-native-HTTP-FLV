import AVFoundation
import CoreMedia
import CryptoKit
import Foundation
import VideoToolbox

/// Live HTTP-FLV player for Streamax dashcams. Production-grade:
///   - Demuxing via the Rust streamax-core via [`StreamaxCore`]
///   - Hardware HEVC decode via AVSampleBufferDisplayLayer
///   - AAC audio decode via AVSampleBufferAudioRenderer (iOS 14+)
///   - Audio/video sync via AVSampleBufferRenderSynchronizer
///   - Reconnect with exponential backoff
///   - Optional SPKI-hash certificate pinning
@available(iOS 14.0, *)
final class StreamaxPlayer: NSObject {

    // MARK: Public

    /// SwiftUI wraps this layer for display.
    let displayLayer = AVSampleBufferDisplayLayer()
    /// Output renderer for AAC audio.
    let audioRenderer = AVSampleBufferAudioRenderer()
    /// Drives both renderers off a single clock.
    let synchronizer = AVSampleBufferRenderSynchronizer()

    /// SHA-256 hashes (Base64) of trusted server SPKIs. When non-empty,
    /// the URLSession delegate rejects any cert whose SPKI hash isn't here.
    var pinnedSPKIHashes: Set<String> = []
    /// Fallback when pinning is empty — trust this exact host's cert without CA verification.
    var trustedInsecureHosts: Set<String> = ["YOUR-CAMERA-HOST.example.com"]

    /// Called on the main queue.
    var onStateChange: ((State) -> Void)?
    /// Called on the main queue.
    var onError: ((String) -> Void)?
    /// Decoded video resolution, set when the SPS arrives.
    private(set) var videoSize: CGSize = .zero

    enum State { case idle, connecting, playing, reconnecting, stopped, error }

    // MARK: Internals

    private let queue = DispatchQueue(label: "streamax.player", qos: .userInitiated)
    private let core = StreamaxCore()
    private lazy var session: URLSession = {
        let cfg = URLSessionConfiguration.ephemeral
        cfg.timeoutIntervalForRequest = 10
        cfg.waitsForConnectivity = false
        return URLSession(configuration: cfg, delegate: self, delegateQueue: nil)
    }()
    private var task: URLSessionDataTask?
    private var currentURL: URL?
    private var videoFormat: CMVideoFormatDescription?
    private var audioFormat: CMAudioFormatDescription?
    private var stopped = true
    private var reconnectAttempt = 0
    private var firstPTS: UInt32?

    override init() {
        super.init()
        synchronizer.addRenderer(displayLayer)
        synchronizer.addRenderer(audioRenderer)
        displayLayer.videoGravity = .resizeAspect
    }

    func play(url: URL) {
        queue.async { [weak self] in
            guard let self = self else { return }
            self.stopped = false
            self.reconnectAttempt = 0
            self.currentURL = url
            self.startTask()
        }
    }

    func stop() {
        queue.async { [weak self] in
            guard let self = self else { return }
            self.stopped = true
            self.task?.cancel()
            self.task = nil
            self.core.reset()
            self.videoFormat = nil
            self.audioFormat = nil
            self.firstPTS = nil
            DispatchQueue.main.async {
                self.synchronizer.setRate(0, time: .zero)
                self.displayLayer.flushAndRemoveImage()
                self.audioRenderer.flush()
                self.fire(.stopped)
            }
        }
    }

    // MARK: Connection lifecycle

    private func startTask() {
        guard let url = currentURL, !stopped else { return }
        DispatchQueue.main.async { self.fire(.connecting) }
        var req = URLRequest(url: url)
        req.setValue("Mozilla/5.0", forHTTPHeaderField: "User-Agent")
        req.setValue("video/x-flv", forHTTPHeaderField: "Accept")
        let t = session.dataTask(with: req)
        task = t
        t.resume()
    }

    private func scheduleReconnect() {
        guard !stopped else { return }
        reconnectAttempt += 1
        let delay = min(pow(2.0, Double(reconnectAttempt - 1)), 30.0) // 1, 2, 4, 8, 16, 30, 30…
        DispatchQueue.main.async { self.fire(.reconnecting) }
        queue.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self = self, !self.stopped else { return }
            self.core.reset()
            self.videoFormat = nil
            self.audioFormat = nil
            self.firstPTS = nil
            self.startTask()
        }
    }

    private func fire(_ state: State) { onStateChange?(state) }

    // MARK: Sample buffer construction

    private func makeVideoFormat(from annexB: Data) -> CMVideoFormatDescription? {
        // Split Annex-B by start codes (0x00000001) into VPS, SPS, PPS.
        var nalus: [Data] = []
        var i = 0
        let bytes = [UInt8](annexB)
        while i + 4 <= bytes.count {
            if bytes[i] == 0 && bytes[i + 1] == 0 && bytes[i + 2] == 0 && bytes[i + 3] == 1 {
                let start = i + 4
                var end = bytes.count
                var j = start
                while j + 3 < bytes.count {
                    if bytes[j] == 0 && bytes[j + 1] == 0 && bytes[j + 2] == 0 && bytes[j + 3] == 1 {
                        end = j; break
                    }
                    j += 1
                }
                nalus.append(Data(bytes[start..<end]))
                i = end
            } else { i += 1 }
        }
        guard nalus.count >= 3 else { return nil }
        let vps = nalus[0], sps = nalus[1], pps = nalus[2]

        let sizes = [vps.count, sps.count, pps.count]
        var fmt: CMVideoFormatDescription?
        let status: OSStatus = vps.withUnsafeBytes { vpsBuf in
            sps.withUnsafeBytes { spsBuf in
                pps.withUnsafeBytes { ppsBuf in
                    let pointers: [UnsafePointer<UInt8>] = [
                        vpsBuf.bindMemory(to: UInt8.self).baseAddress!,
                        spsBuf.bindMemory(to: UInt8.self).baseAddress!,
                        ppsBuf.bindMemory(to: UInt8.self).baseAddress!,
                    ]
                    return pointers.withUnsafeBufferPointer { p in
                        sizes.withUnsafeBufferPointer { s in
                            CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                                allocator: kCFAllocatorDefault,
                                parameterSetCount: 3,
                                parameterSetPointers: p.baseAddress!,
                                parameterSetSizes: s.baseAddress!,
                                nalUnitHeaderLength: 4,
                                extensions: nil,
                                formatDescriptionOut: &fmt
                            )
                        }
                    }
                }
            }
        }
        return status == noErr ? fmt : nil
    }

    /// Convert Annex-B NALUs (00 00 00 01 ...) back to AVCC (length-prefixed)
    /// for CMSampleBuffer.
    private func annexBToAVCC(_ annexB: Data) -> Data {
        var out = Data(capacity: annexB.count)
        var i = 0
        let bytes = [UInt8](annexB)
        while i + 4 <= bytes.count {
            if !(bytes[i] == 0 && bytes[i + 1] == 0 && bytes[i + 2] == 0 && bytes[i + 3] == 1) {
                i += 1; continue
            }
            let start = i + 4
            var end = bytes.count
            var j = start
            while j + 3 < bytes.count {
                if bytes[j] == 0 && bytes[j + 1] == 0 && bytes[j + 2] == 0 && bytes[j + 3] == 1 {
                    end = j; break
                }
                j += 1
            }
            let len = UInt32(end - start).bigEndian
            withUnsafeBytes(of: len) { out.append(contentsOf: $0) }
            out.append(contentsOf: bytes[start..<end])
            i = end
        }
        return out
    }

    private func makeVideoSampleBuffer(avcc: Data, fmt: CMVideoFormatDescription,
                                        ptsMs: UInt32, isKeyframe: Bool) -> CMSampleBuffer? {
        let length = avcc.count
        var bb: CMBlockBuffer?
        guard CMBlockBufferCreateWithMemoryBlock(
            allocator: kCFAllocatorDefault, memoryBlock: nil, blockLength: length,
            blockAllocator: kCFAllocatorDefault, customBlockSource: nil,
            offsetToData: 0, dataLength: length, flags: 0, blockBufferOut: &bb
        ) == noErr, let bb = bb else { return nil }

        let copy = avcc.withUnsafeBytes { raw -> OSStatus in
            CMBlockBufferReplaceDataBytes(with: raw.baseAddress!, blockBuffer: bb,
                                          offsetIntoDestination: 0, dataLength: length)
        }
        guard copy == noErr else { return nil }

        var timing = CMSampleTimingInfo(
            duration: CMTime(value: 1, timescale: 25),
            presentationTimeStamp: CMTime(value: CMTimeValue(ptsMs), timescale: 1000),
            decodeTimeStamp: .invalid
        )
        var size = length
        var sb: CMSampleBuffer?
        guard CMSampleBufferCreateReady(
            allocator: kCFAllocatorDefault, dataBuffer: bb, formatDescription: fmt,
            sampleCount: 1, sampleTimingEntryCount: 1, sampleTimingArray: &timing,
            sampleSizeEntryCount: 1, sampleSizeArray: &size, sampleBufferOut: &sb
        ) == noErr else { return nil }

        if let attachments = CMSampleBufferGetSampleAttachmentsArray(sb!, createIfNecessary: true)
            as? [NSMutableDictionary], let f = attachments.first {
            if !isKeyframe { f[kCMSampleAttachmentKey_NotSync as String] = true }
        }
        return sb
    }

    private func makeAudioFormat(config: Data, sampleRate: Int, channels: Int, objectType: Int) -> CMAudioFormatDescription? {
        var asbd = AudioStreamBasicDescription(
            mSampleRate: Float64(sampleRate),
            mFormatID: kAudioFormatMPEG4AAC,
            mFormatFlags: AudioFormatFlags(objectType),
            mBytesPerPacket: 0, mFramesPerPacket: 1024, mBytesPerFrame: 0,
            mChannelsPerFrame: UInt32(channels), mBitsPerChannel: 0, mReserved: 0
        )
        var fmt: CMAudioFormatDescription?
        // Magic cookie is the AudioSpecificConfig prepended with an ESDS-like wrapper;
        // for AAC the simplest path is to pass the raw ASC as the magic cookie.
        var status: OSStatus = noErr
        config.withUnsafeBytes { raw in
            status = CMAudioFormatDescriptionCreate(
                allocator: kCFAllocatorDefault, asbd: &asbd,
                layoutSize: 0, layout: nil,
                magicCookieSize: config.count, magicCookie: raw.baseAddress,
                extensions: nil, formatDescriptionOut: &fmt
            )
        }
        return status == noErr ? fmt : nil
    }

    private func makeAudioSampleBuffer(data: Data, fmt: CMAudioFormatDescription,
                                        ptsMs: UInt32) -> CMSampleBuffer? {
        let length = data.count
        var bb: CMBlockBuffer?
        guard CMBlockBufferCreateWithMemoryBlock(
            allocator: kCFAllocatorDefault, memoryBlock: nil, blockLength: length,
            blockAllocator: kCFAllocatorDefault, customBlockSource: nil,
            offsetToData: 0, dataLength: length, flags: 0, blockBufferOut: &bb
        ) == noErr, let bb = bb else { return nil }

        let copy = data.withUnsafeBytes { raw -> OSStatus in
            CMBlockBufferReplaceDataBytes(with: raw.baseAddress!, blockBuffer: bb,
                                          offsetIntoDestination: 0, dataLength: length)
        }
        guard copy == noErr else { return nil }

        var timing = CMSampleTimingInfo(
            duration: CMTime(value: 1024, timescale: CMTimeScale(getSampleRate(fmt))),
            presentationTimeStamp: CMTime(value: CMTimeValue(ptsMs), timescale: 1000),
            decodeTimeStamp: .invalid
        )
        var size = length
        var sb: CMSampleBuffer?
        guard CMSampleBufferCreateReady(
            allocator: kCFAllocatorDefault, dataBuffer: bb, formatDescription: fmt,
            sampleCount: 1, sampleTimingEntryCount: 1, sampleTimingArray: &timing,
            sampleSizeEntryCount: 1, sampleSizeArray: &size, sampleBufferOut: &sb
        ) == noErr else { return nil }
        return sb
    }

    private func getSampleRate(_ fmt: CMAudioFormatDescription) -> Int {
        guard let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(fmt) else { return 44100 }
        return Int(asbd.pointee.mSampleRate)
    }

    // MARK: Demuxer drain

    fileprivate func drainEvents() {
        while let ev = core.nextEvent() {
            switch ev {
            case .videoConfig(let annexB, let w, let h):
                if let fmt = makeVideoFormat(from: annexB) {
                    videoFormat = fmt
                    videoSize = CGSize(width: w, height: h)
                }
            case .videoFrame(let annexB, let pts, let isKey):
                guard let fmt = videoFormat else { continue }
                let avcc = annexBToAVCC(annexB)
                guard let sb = makeVideoSampleBuffer(avcc: avcc, fmt: fmt,
                                                     ptsMs: pts, isKeyframe: isKey) else { continue }
                if firstPTS == nil { firstPTS = pts; startSynchronizer(at: pts) }
                DispatchQueue.main.async {
                    self.displayLayer.enqueue(sb)
                    if self.synchronizer.rate == 0 { self.fire(.playing) }
                }
            case .audioConfig(let cfg, let sr, let ch, let ot):
                audioFormat = makeAudioFormat(config: cfg, sampleRate: sr, channels: ch, objectType: ot)
            case .audioFrame(let data, let pts):
                guard let fmt = audioFormat else { continue }
                guard let sb = makeAudioSampleBuffer(data: data, fmt: fmt, ptsMs: pts) else { continue }
                DispatchQueue.main.async { self.audioRenderer.enqueue(sb) }
            case .error(let msg):
                DispatchQueue.main.async {
                    self.onError?(msg)
                    self.fire(.error)
                }
            }
        }
    }

    private func startSynchronizer(at pts: UInt32) {
        let start = CMTime(value: CMTimeValue(pts), timescale: 1000)
        DispatchQueue.main.async {
            self.synchronizer.setRate(1.0, time: start)
        }
    }
}

// MARK: URLSessionDataDelegate

@available(iOS 14.0, *)
extension StreamaxPlayer: URLSessionDataDelegate {

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        core.append(data)
        drainEvents()
    }

    func urlSession(_ session: URLSession,
                    dataTask: URLSessionDataTask,
                    didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            completionHandler(.cancel)
            queue.async { self.scheduleReconnect() }
            return
        }
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession,
                    didReceive challenge: URLAuthenticationChallenge,
                    completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
        guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
              let trust = challenge.protectionSpace.serverTrust else {
            completionHandler(.performDefaultHandling, nil); return
        }
        let host = challenge.protectionSpace.host

        // 1. SPKI pinning takes priority when configured.
        if !pinnedSPKIHashes.isEmpty {
            if certChainMatchesPinned(trust) {
                completionHandler(.useCredential, URLCredential(trust: trust))
            } else {
                completionHandler(.cancelAuthenticationChallenge, nil)
            }
            return
        }

        // 2. Otherwise, accept the cert from the trusted insecure-host list.
        if trustedInsecureHosts.contains(host) {
            completionHandler(.useCredential, URLCredential(trust: trust))
        } else {
            completionHandler(.performDefaultHandling, nil)
        }
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        queue.async {
            if self.stopped { return }
            // Successful EOF or any error → reconnect.
            self.scheduleReconnect()
        }
    }

    private func certChainMatchesPinned(_ trust: SecTrust) -> Bool {
        let count = SecTrustGetCertificateCount(trust)
        for i in 0..<count {
            guard let cert = SecTrustGetCertificateAtIndex(trust, i) else { continue }
            guard let pubKey = SecCertificateCopyKey(cert),
                  let pubKeyData = SecKeyCopyExternalRepresentation(pubKey, nil) as Data?
            else { continue }
            let hash = Data(SHA256.hash(data: pubKeyData))
            let b64 = hash.base64EncodedString()
            if pinnedSPKIHashes.contains(b64) { return true }
        }
        return false
    }
}
