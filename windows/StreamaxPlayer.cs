using System;
using System.Collections.Concurrent;
using System.IO;
using System.Net.Http;
using System.Net.Security;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using System.Threading;
using System.Threading.Tasks;
using LibVLCSharp.Shared;

namespace StreamaxDemo;

/// <summary>
/// Live HTTP-FLV player. Demux in Rust (streamax-core), HEVC playback via
/// libVLC. Reconnects with exponential backoff. Supports SPKI pinning.
///
/// AUDIO: this build is video-only on Windows. AAC audio extraction works
/// (the Rust core hands us AudioConfig + AudioFrame events) but libVLC
/// can't multiplex two raw elementary streams. To enable audio:
///   (a) extend streamax-core with an MPEG-TS muxer (HEVC + AAC → TS),
///   (b) replace `:demux=hevc` with the TS feed.
/// See the chat notes for the design.
/// </summary>
public sealed class StreamaxPlayer : IDisposable
{
    public event Action<PlayerState>? StateChanged;
    public event Action<string>? ErrorOccurred;
    public event Action<int, int>? VideoSizeChanged;

    public MediaPlayer MediaPlayer { get; }

    public enum PlayerState { Idle, Connecting, Playing, Reconnecting, Stopped, Error }

    /// <summary>
    /// Base64 SHA-256 of trusted server SPKIs. Non-empty enables pinning;
    /// any cert whose SPKI hash isn't in this set is rejected.
    /// </summary>
    public string[] PinnedSpkiHashesBase64 { get; set; } = Array.Empty<string>();
    /// <summary>Fallback when pinning is empty.</summary>
    public string[] TrustedInsecureHosts { get; set; } = { "YOUR-CAMERA-HOST.example.com" };

    private readonly LibVLC _libVlc;
    private readonly StreamaxCore _core = new();
    private readonly HttpClient _http;
    private CancellationTokenSource? _cts;
    private HevcStream? _videoStream;
    private Media? _media;
    private int _reconnectAttempt;
    private string? _currentUrl;
    private PlayerState _state = PlayerState.Idle;

    public StreamaxPlayer(LibVLC libVlc)
    {
        _libVlc = libVlc;
        MediaPlayer = new MediaPlayer(libVlc);

        var handler = new HttpClientHandler
        {
            ServerCertificateCustomValidationCallback = ValidateCert
        };
        _http = new HttpClient(handler) { Timeout = Timeout.InfiniteTimeSpan };
    }

    public Task PlayAsync(string url)
    {
        Stop();
        _currentUrl = url;
        _reconnectAttempt = 0;
        return StartAsync();
    }

    public void Stop()
    {
        _cts?.Cancel(); _cts = null;
        MediaPlayer.Stop();
        _media?.Dispose(); _media = null;
        _videoStream?.Complete(); _videoStream?.Dispose(); _videoStream = null;
        _core.Reset();
        if (_state != PlayerState.Error) SetState(PlayerState.Stopped);
    }

    public void Dispose()
    {
        Stop();
        MediaPlayer.Dispose();
        _core.Dispose();
        _http.Dispose();
    }

    private async Task StartAsync()
    {
        if (_currentUrl == null) return;
        SetState(PlayerState.Connecting);

        var cts = new CancellationTokenSource();
        _cts = cts;
        _videoStream = new HevcStream();
        _media = new Media(_libVlc, new StreamMediaInput(_videoStream), ":demux=hevc");
        MediaPlayer.Play(_media);

        try
        {
            using var req = new HttpRequestMessage(HttpMethod.Get, _currentUrl);
            req.Headers.UserAgent.ParseAdd("Mozilla/5.0");
            using var resp = await _http.SendAsync(req, HttpCompletionOption.ResponseHeadersRead, cts.Token);
            resp.EnsureSuccessStatusCode();

            using var body = await resp.Content.ReadAsStreamAsync(cts.Token);
            var buf = new byte[64 * 1024];
            SetState(PlayerState.Playing);
            _reconnectAttempt = 0;

            while (!cts.IsCancellationRequested)
            {
                int n = await body.ReadAsync(buf.AsMemory(0, buf.Length), cts.Token);
                if (n <= 0) break;
                _core.Append(buf.AsSpan(0, n));
                DrainEvents();
            }
        }
        catch (OperationCanceledException) { return; }
        catch (Exception e)
        {
            ErrorOccurred?.Invoke($"stream error: {e.Message}");
        }
        finally { _videoStream?.Complete(); }

        if (!cts.IsCancellationRequested) ScheduleReconnect();
    }

    private void DrainEvents()
    {
        while (true)
        {
            var ev = _core.NextEvent();
            if (ev == null) break;
            switch (ev)
            {
                case StreamaxCore.VideoConfig vc:
                    VideoSizeChanged?.Invoke(vc.Width, vc.Height);
                    _videoStream?.Push(vc.AnnexB);
                    break;
                case StreamaxCore.VideoFrame vf:
                    _videoStream?.Push(vf.AnnexB);
                    break;
                case StreamaxCore.AudioConfig _:
                case StreamaxCore.AudioFrame _:
                    // See class doc — audio path not wired on Windows yet.
                    break;
                case StreamaxCore.Error err:
                    ErrorOccurred?.Invoke(err.Message);
                    SetState(PlayerState.Error);
                    break;
            }
        }
    }

    private async void ScheduleReconnect()
    {
        if (_currentUrl == null) return;
        _reconnectAttempt += 1;
        var delaySec = Math.Min(Math.Pow(2, _reconnectAttempt - 1), 30);
        SetState(PlayerState.Reconnecting);
        try { await Task.Delay(TimeSpan.FromSeconds(delaySec)); }
        catch { return; }
        _core.Reset();
        await StartAsync();
    }

    private bool ValidateCert(HttpRequestMessage msg, X509Certificate2? cert,
                               X509Chain? chain, SslPolicyErrors errors)
    {
        if (cert == null) return false;

        // 1. SPKI pinning takes priority.
        if (PinnedSpkiHashesBase64.Length > 0)
        {
            var spkiDer = cert.PublicKey.ExportSubjectPublicKeyInfo();
            var hash = SHA256.HashData(spkiDer);
            var hashB64 = Convert.ToBase64String(hash);
            return Array.IndexOf(PinnedSpkiHashesBase64, hashB64) >= 0;
        }

        // 2. Fallback: trust by hostname.
        var host = msg.RequestUri?.Host;
        if (host != null && Array.IndexOf(TrustedInsecureHosts, host) >= 0) return true;

        return errors == SslPolicyErrors.None;
    }

    private void SetState(PlayerState s) { _state = s; StateChanged?.Invoke(s); }
}

/// <summary>
/// Read-only Stream that emits queued byte arrays to libVLC. Blocking Take
/// gives natural backpressure; Complete unblocks libVLC with EOF.
/// </summary>
internal sealed class HevcStream : Stream
{
    private readonly BlockingCollection<byte[]> _queue = new(boundedCapacity: 256);
    private byte[]? _current;
    private int _offset;

    public void Push(byte[] data) { try { _queue.Add(data); } catch { } }
    public void Complete() { try { _queue.CompleteAdding(); } catch { } }

    public override bool CanRead => true;
    public override bool CanSeek => false;
    public override bool CanWrite => false;
    public override long Length => throw new NotSupportedException();
    public override long Position { get => throw new NotSupportedException(); set => throw new NotSupportedException(); }
    public override void Flush() { }
    public override long Seek(long _, SeekOrigin __) => throw new NotSupportedException();
    public override void SetLength(long _) => throw new NotSupportedException();
    public override void Write(byte[] _, int __, int ___) => throw new NotSupportedException();

    public override int Read(byte[] buffer, int offset, int count)
    {
        if (_current == null || _offset >= _current.Length)
        {
            try { _current = _queue.Take(); _offset = 0; }
            catch (InvalidOperationException) { return 0; }
        }
        int n = Math.Min(count, _current.Length - _offset);
        Buffer.BlockCopy(_current, _offset, buffer, offset, n);
        _offset += n;
        return n;
    }

    protected override void Dispose(bool disposing)
    {
        if (disposing) { try { _queue.Dispose(); } catch { } }
        base.Dispose(disposing);
    }
}
