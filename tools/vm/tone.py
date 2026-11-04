#!/usr/bin/env python3
"""A tone played through PulseAudio (libpulse-simple, which the wadspace base
has; it has no player of its own). tools/vm.sh check runs it inside a
wadspace: PULSE_SERVER, from its unit, is the machine's PipeWire.

    tone.py [SECONDS] [HZ]
"""

import ctypes
import math
import struct
import sys

PA_SAMPLE_S16LE = 3
PA_STREAM_PLAYBACK = 1
RATE = 44100


class SampleSpec(ctypes.Structure):
    _fields_ = [("format", ctypes.c_int), ("rate", ctypes.c_uint32), ("channels", ctypes.c_uint8)]


def main():
    seconds = float(sys.argv[1]) if len(sys.argv) > 1 else 3.0
    hz = float(sys.argv[2]) if len(sys.argv) > 2 else 440.0
    simple = ctypes.CDLL("libpulse-simple.so.0")
    pulse = ctypes.CDLL("libpulse.so.0")
    pulse.pa_strerror.restype = ctypes.c_char_p
    simple.pa_simple_new.restype = ctypes.c_void_p
    simple.pa_simple_new.argtypes = [
        ctypes.c_char_p, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_char_p,
        ctypes.POINTER(SampleSpec), ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_int),
    ]
    simple.pa_simple_write.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_int)]
    simple.pa_simple_drain.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int)]
    simple.pa_simple_free.argtypes = [ctypes.c_void_p]

    spec = SampleSpec(PA_SAMPLE_S16LE, RATE, 2)
    err = ctypes.c_int(0)
    # No server named: PULSE_SERVER says which.
    s = simple.pa_simple_new(None, b"wados-check", PA_STREAM_PLAYBACK, None, b"tone", ctypes.byref(spec), None, None,
                             ctypes.byref(err))
    if not s:
        sys.exit(f"tone.py: no PulseAudio: {pulse.pa_strerror(err.value).decode()}")
    frames = int(seconds * RATE)
    amp = 0.8 * 32767
    data = b"".join(
        struct.pack("<hh", v, v) for v in (int(amp * math.sin(2 * math.pi * hz * i / RATE)) for i in range(frames))
    )
    if simple.pa_simple_write(s, data, len(data), ctypes.byref(err)) < 0 or simple.pa_simple_drain(s, ctypes.byref(err)) < 0:
        sys.exit(f"tone.py: playing: {pulse.pa_strerror(err.value).decode()}")
    simple.pa_simple_free(s)
    print(f"played {seconds:g} s of {hz:g} Hz")


if __name__ == "__main__":
    main()
