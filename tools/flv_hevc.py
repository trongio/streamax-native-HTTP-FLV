#!/usr/bin/env python3
"""
Streamax HTTP-FLV (HEVC variant) → raw HEVC Annex-B on stdout.

Mainline FFmpeg's FLV demuxer doesn't handle codec_id=12 (HEVC) — the
non-standard Chinese variant Streamax/Hikvision/etc. use. We parse FLV
ourselves and emit Annex-B NALUs that any HEVC decoder can play.

Usage (recommended — let curl handle HTTPS/CA trust):
  curl -sN 'URL' | python3 flv_hevc.py - | mpv --demuxer=lavf --demuxer-lavf-format=hevc -

Or have the script fetch the URL itself (uses Python's TLS stack):
  python3 flv_hevc.py URL | mpv --demuxer=lavf --demuxer-lavf-format=hevc -
  python3 flv_hevc.py --insecure URL | mpv ...    # skip cert verification

Record to file:
  curl -sN 'URL' | python3 flv_hevc.py - | ffmpeg -f hevc -i - -c copy -f matroska out.mkv
"""
from __future__ import annotations

import os
import ssl
import struct
import sys
import urllib.request

START_CODE = b"\x00\x00\x00\x01"


def parse_hvcc(data: bytes) -> list[bytes]:
    """Parse HEVCDecoderConfigurationRecord → list of NALUs (VPS, SPS, PPS)."""
    if len(data) < 23 or data[0] != 1:
        raise ValueError("not an hvcC record")
    p = 22
    num_arrays = data[p]
    p += 1
    nalus: list[bytes] = []
    for _ in range(num_arrays):
        p += 1  # completeness + nal_type byte
        (num_nalus,) = struct.unpack_from(">H", data, p)
        p += 2
        for _ in range(num_nalus):
            (ln,) = struct.unpack_from(">H", data, p)
            p += 2
            nalus.append(data[p : p + ln])
            p += ln
    return nalus


def emit_length_prefixed(payload: bytes, out) -> None:
    """4-byte big-endian length-prefixed NALUs → Annex-B."""
    i = 0
    n = len(payload)
    while i + 4 <= n:
        (ln,) = struct.unpack_from(">I", payload, i)
        i += 4
        if i + ln > n:
            return
        out.write(START_CODE)
        out.write(payload[i : i + ln])
        i += ln


def _open_source(args: list[str]):
    if not args:
        sys.stderr.write(
            f"usage: {sys.argv[0]} [--insecure] (URL | -)\n"
            "  URL  fetch over HTTPS using Python's TLS\n"
            "  -    read from stdin (pipe in via curl)\n"
        )
        sys.exit(1)

    insecure = "--insecure" in args
    args = [a for a in args if a != "--insecure"]
    target = args[0]

    if target == "-":
        return sys.stdin.buffer

    ctx = ssl.create_default_context()
    # Try common Linux CA bundle locations if Python's default isn't trusted
    for cafile in (
        os.environ.get("SSL_CERT_FILE"),
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
        "/etc/ssl/cert.pem",
    ):
        if cafile and os.path.isfile(cafile):
            ctx = ssl.create_default_context(cafile=cafile)
            break
    if insecure:
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
        sys.stderr.write("WARNING: TLS verification disabled\n")

    req = urllib.request.Request(target, headers={"User-Agent": "flv-hevc/1.0"})
    return urllib.request.urlopen(req, timeout=10, context=ctx)


def main() -> int:
    resp = _open_source(sys.argv[1:])
    out = sys.stdout.buffer

    header = resp.read(9)
    if header[:3] != b"FLV":
        sys.stderr.write("not an FLV stream\n")
        return 2
    resp.read(4)  # PreviousTagSize0

    tags = 0
    video_tags = 0
    while True:
        tag_header = resp.read(11)
        if len(tag_header) < 11:
            break
        tag_type = tag_header[0] & 0x1F
        data_size = (tag_header[1] << 16) | (tag_header[2] << 8) | tag_header[3]
        body = resp.read(data_size)
        if len(body) < data_size:
            break
        resp.read(4)  # PreviousTagSize
        tags += 1

        if tag_type != 9 or data_size < 5:
            continue
        codec_id = body[0] & 0x0F
        if codec_id != 12:  # 12 = HEVC (non-standard FLV)
            if tags < 5:
                sys.stderr.write(f"non-HEVC video tag, codec_id={codec_id}\n")
            continue

        packet_type = body[1]
        payload = body[5:]
        video_tags += 1

        if packet_type == 0:
            try:
                for nalu in parse_hvcc(payload):
                    out.write(START_CODE)
                    out.write(nalu)
            except Exception as exc:
                sys.stderr.write(f"hvcC parse failed: {exc}\n")
                continue
        elif packet_type == 1:
            emit_length_prefixed(payload, out)
        # packet_type 2 = end of sequence — ignored
        out.flush()

    sys.stderr.write(f"done: {tags} tags, {video_tags} HEVC video tags\n")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        pass
    except BrokenPipeError:
        pass
