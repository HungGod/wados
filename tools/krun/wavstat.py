"""How loud a WAV file is: the check's recording of the VM's null sink (prototype.sh check)."""
import array
import math
import sys
import wave

w = wave.open(sys.argv[1])
n = w.getnframes()
a = array.array("h", w.readframes(n))
rms = math.sqrt(sum(x * x for x in a) / max(len(a), 1))
print(f"   the null sink's monitor, recorded in the VM: {n / w.getframerate():.2f} s, "
      f"{w.getnchannels()} ch, RMS {rms:.0f} of 32768 (silence: 0)")
