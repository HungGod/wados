"""try-pads.sh's players: each signs in with its own token and presses a
button on its own pad. Player 2 also tries to press player 1's: Selkies drops it.

    driver.py GUID KEY1 KEY2 KEY3 KEY4
"""
import asyncio
import base64
import sys

import aiohttp

GUID = sys.argv[1]
NAME = base64.b64encode(b"Xbox 360 Controller").decode()


async def player(http, slot, key):
    ws = await http.ws_connect(f"http://127.0.0.1:3000/ws-{GUID}/api/websockets?token={key}", max_msg_size=0)
    idx = slot - 1
    await ws.send_str(f"js,c,{idx},{NAME},8,16")
    await asyncio.sleep(1)
    await ws.send_str(f"js,b,{idx},{slot},1")
    await asyncio.sleep(0.2)
    await ws.send_str(f"js,b,{idx},{slot},0")
    if slot == 2:
        # Someone else's pad.
        await ws.send_str("js,b,0,9,1")
    await asyncio.sleep(1)
    return ws


async def main():
    async with aiohttp.ClientSession() as http:
        socks = [await player(http, i + 1, k) for i, k in enumerate(sys.argv[2:6])]
        await asyncio.sleep(1)
        for s in socks:
            await s.close()


asyncio.run(main())
