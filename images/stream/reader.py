"""try-pads.sh's game: reads all four pads through the interposer, as a game
reads /dev/input/js*, and prints each button press: `pad <n> button <b>`."""
import os
import select
import struct
import sys

pads = {}
for i in range(4):
    pads[os.open(f"/dev/input/js{i}", os.O_RDONLY | os.O_NONBLOCK)] = i
print("open", flush=True)
while True:
    ready, _, _ = select.select(list(pads), [], [])
    for fd in ready:
        try:
            data = os.read(fd, 64)
        except BlockingIOError:
            continue
        for off in range(0, len(data) - 7, 8):
            _, value, kind, number = struct.unpack_from("IhBB", data, off)
            # 0x80 is the state a pad reports as it's opened.
            if kind == 1 and value:
                print(f"pad {pads[fd]} button {number}", flush=True)
