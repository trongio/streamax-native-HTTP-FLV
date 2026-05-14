using System;
using System.Runtime.InteropServices;
using System.Text;

namespace StreamaxDemo;

/// <summary>
/// P/Invoke wrapper for streamax-core (streamax_core.dll). Pull API.
/// Single-threaded: serialize calls or use one instance per thread.
/// Place streamax_core.dll next to the executable (or in the standard
/// DllImport search path).
/// </summary>
public sealed class StreamaxCore : IDisposable
{
    public abstract record Event;
    public sealed record VideoConfig(byte[] AnnexB, int Width, int Height) : Event;
    public sealed record VideoFrame(byte[] AnnexB, uint PtsMs, bool IsKeyframe) : Event;
    public sealed record AudioConfig(byte[] Config, int SampleRate, int Channels, int ObjectType) : Event;
    public sealed record AudioFrame(byte[] Data, uint PtsMs) : Event;
    public sealed record Error(string Message) : Event;

    private IntPtr _handle;
    private bool _disposed;

    public StreamaxCore() { _handle = streamax_demuxer_new(); }

    public void Reset()
    {
        if (_handle != IntPtr.Zero) streamax_demuxer_reset(_handle);
    }

    public void Append(ReadOnlySpan<byte> chunk)
    {
        if (_handle == IntPtr.Zero || chunk.IsEmpty) return;
        unsafe
        {
            fixed (byte* p = chunk)
            {
                streamax_demuxer_append(_handle, (IntPtr)p, (nuint)chunk.Length);
            }
        }
    }

    public Event? NextEvent()
    {
        if (_handle == IntPtr.Zero) return null;
        var ev = new CEvent();
        if (streamax_demuxer_next_event(_handle, ref ev) == 0) return null;

        var data = ev.data_len > 0 && ev.data != IntPtr.Zero
            ? CopyBytes(ev.data, (int)ev.data_len)
            : Array.Empty<byte>();

        return ev.kind switch
        {
            EventKind.VideoConfig => new VideoConfig(data, (int)ev.width, (int)ev.height),
            EventKind.VideoFrame  => new VideoFrame(data, ev.pts_ms, ev.is_keyframe != 0),
            EventKind.AudioConfig => new AudioConfig(data, (int)ev.sample_rate, ev.channels, ev.object_type),
            EventKind.AudioFrame  => new AudioFrame(data, ev.pts_ms),
            EventKind.Error       => new Error(Encoding.UTF8.GetString(data)),
            _ => null,
        };
    }

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;
        if (_handle != IntPtr.Zero) { streamax_demuxer_free(_handle); _handle = IntPtr.Zero; }
        GC.SuppressFinalize(this);
    }

    ~StreamaxCore() { Dispose(); }

    private static byte[] CopyBytes(IntPtr ptr, int len)
    {
        var buf = new byte[len];
        Marshal.Copy(ptr, buf, 0, len);
        return buf;
    }

    // ---- P/Invoke ----

    private enum EventKind : int
    {
        None = 0, VideoConfig = 1, VideoFrame = 2, AudioConfig = 3, AudioFrame = 4, Error = 99,
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CEvent
    {
        public EventKind kind;
        public IntPtr    data;
        public nuint     data_len;
        public uint      pts_ms;
        public byte      is_keyframe;
        public uint      width;
        public uint      height;
        public uint      sample_rate;
        public byte      channels;
        public byte      object_type;
    }

    private const string DLL = "streamax_core";

    [DllImport(DLL, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr streamax_demuxer_new();

    [DllImport(DLL, CallingConvention = CallingConvention.Cdecl)]
    private static extern void streamax_demuxer_free(IntPtr handle);

    [DllImport(DLL, CallingConvention = CallingConvention.Cdecl)]
    private static extern void streamax_demuxer_reset(IntPtr handle);

    [DllImport(DLL, CallingConvention = CallingConvention.Cdecl)]
    private static extern void streamax_demuxer_append(IntPtr handle, IntPtr data, nuint len);

    [DllImport(DLL, CallingConvention = CallingConvention.Cdecl)]
    private static extern byte streamax_demuxer_next_event(IntPtr handle, ref CEvent outEvent);
}
