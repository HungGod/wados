#!/usr/bin/env python3
"""Was anything heard? QEMU's wav audiodev records what the VM's sound card
plays into a file, which grows while it runs (its header's sizes are only
right once QEMU exits). This reads the samples written after byte FROM.

    wav.py FILE [FROM]      prints what it found; exit 0 when it heard
                            at least MIN_LOUD_S of sound, 1 when not

Standard library only.
"""

import math
import struct
import sys

MIN_LOUD_S = 0.5
# A 16-bit window this loud (RMS) isn't silence or dither.
LOUD_RMS = 100
WINDOW_S = 0.05


def fmt_and_data(raw):
    """(channels, rate, bits, offset of the samples)."""
    if raw[:4] != b"RIFF" or raw[8:12] != b"WAVE":
        raise SystemExit("wav.py: not a WAV file")
    pos, fmt = 12, None
    while pos + 8 <= len(raw):
        cid, size = raw[pos:pos + 4], struct.unpack("<I", raw[pos + 4:pos + 8])[0]
        if cid == b"fmt ":
            _, channels, rate, _, _, bits = struct.unpack("<HHIIHH", raw[pos + 8:pos + 24])
            fmt = (channels, rate, bits)
        elif cid == b"data":
            if fmt is None:
                raise SystemExit("wav.py: no fmt chunk before the data")
            return (*fmt, pos + 8)
        pos += 8 + size
    raise SystemExit("wav.py: no data chunk")


def main(argv):
    if len(argv) not in (2, 3):
        raise SystemExit(__doc__)
    with open(argv[1], "rb") as f:
        raw = f.read()
    channels, rate, bits, data = fmt_and_data(raw)
    if bits != 16:
        raise SystemExit(f"wav.py: {bits}-bit samples (only 16-bit is read)")
    start = max(data, int(argv[2]) if len(argv) == 3 else data)
    frame = channels * 2
    start += (start - data) % frame and frame - (start - data) % frame
    body = raw[start:len(raw) - (len(raw) - start) % frame]
    n = len(body) // 2
    samples = struct.unpack(f"<{n}h", body) if n else ()
    per_window = max(1, int(WINDOW_S * rate)) * channels
    loud = 0
    peak = max((abs(s) for s in samples), default=0)
    for i in range(0, n - per_window + 1, per_window):
        w = samples[i:i + per_window]
        if math.sqrt(sum(s * s for s in w) / len(w)) >= LOUD_RMS:
            loud += 1
    seconds = n / channels / rate
    loud_s = loud * WINDOW_S
    print(f"{seconds:.2f} s recorded since byte {start} ({rate} Hz, {channels} ch): "
          f"{loud_s:.2f} s loud, peak {peak}")
    sys.exit(0 if loud_s >= MIN_LOUD_S else 1)


if __name__ == "__main__":
    main(sys.argv)
