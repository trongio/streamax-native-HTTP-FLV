using System;
using System.Windows;
using LibVLCSharp.Shared;

namespace StreamaxDemo;

public partial class MainWindow : Window
{
    private readonly LibVLC _libVlc;
    private readonly StreamaxPlayer _player;
    private bool _playing;

    public MainWindow()
    {
        Core.Initialize();
        InitializeComponent();

        _libVlc = new LibVLC(
            "--network-caching=300",
            "--no-osd",
            "--quiet"
        );
        _player = new StreamaxPlayer(_libVlc)
        {
            // SHA-256 of the SubjectPublicKeyInfo for YOUR-CAMERA-HOST.example.com:22060
            // (Sectigo-issued cert, expires 2026-12-28). Re-pin if the cert key rotates.
            PinnedSpkiHashesBase64 = new[] { "YOUR-SPKI-HASH-BASE64=" },
        };

        Loaded += (_, _) => VideoView.MediaPlayer = _player.MediaPlayer;

        _player.StateChanged += s => Dispatcher.Invoke(() =>
        {
            StatusText.Text = s.ToString();
            _playing = s is StreamaxPlayer.PlayerState.Playing
                      or StreamaxPlayer.PlayerState.Connecting
                      or StreamaxPlayer.PlayerState.Reconnecting;
            PlayButton.Content = _playing ? "Stop" : "Play";
        });
        _player.VideoSizeChanged += (w, h) => Dispatcher.Invoke(() =>
            StatusText.Text = $"Playing — {w}×{h}");
        _player.ErrorOccurred += msg => Dispatcher.Invoke(() => ErrorText.Text = msg);

        Closed += OnClosed;
    }

    private async void OnPlayClick(object sender, RoutedEventArgs e)
    {
        ErrorText.Text = "";
        if (_playing)
        {
            _player.Stop();
            return;
        }
        var url = UrlBox.Text.Trim();
        if (!Uri.TryCreate(url, UriKind.Absolute, out _))
        {
            ErrorText.Text = "Invalid URL";
            return;
        }
        await _player.PlayAsync(url);
    }

    private void OnClosed(object? sender, EventArgs e)
    {
        _player.Dispose();
        _libVlc.Dispose();
    }
}
