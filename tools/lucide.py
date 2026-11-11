#!/usr/bin/env python3
"""Writes the lucide icons the shell and the apps draw (assets/lucide/<name>.svg,
stroke in currentColor), from a pinned lucide-static release (its npm tarball,
checked against its SHA-512; no Node.js):

    python3 tools/lucide.py [name ...]

Takes the icons already listed in assets/lucide/NAMES plus the names given
(lucide's file names: "circle-check"). wad-gtk compiles them in.
"""

import base64
import hashlib
import io
import re
import sys
import tarfile
import urllib.request
from pathlib import Path

VERSION = "1.56.0"
TARBALL = f"https://registry.npmjs.org/lucide-static/-/lucide-static-{VERSION}.tgz"
SHA512 = "R1W4B9jCNbHmqD2JnJW5+DF4JAecKP7RSd+eMqV2aQxLKcB0MRKz0QNHpMvJtE8EhFX3Qc5V6XmUpiXD2Vcc2w=="

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "assets" / "lucide"
LISTED = OUT / "NAMES"


def tidy(svg: str) -> str:
    """Keep the drawing; drop lucide's comment and class, so every icon reads the same."""
    svg = re.sub(r"<!--[\s\S]*?-->\s*", "", svg)
    svg = re.sub(r'\s+class="[^"]*"', "", svg, count=1)
    svg = re.sub(r"\s*\n\s*", " ", svg)
    svg = re.sub(r">\s+<", "><", svg)
    return svg.strip()


def main(args: list[str]) -> int:
    names = set(args)
    if LISTED.exists():
        for line in LISTED.read_text().splitlines():
            if line and not line.startswith("#"):
                names.add(line.strip())

    with urllib.request.urlopen(TARBALL) as r:
        data = r.read()
    got = base64.b64encode(hashlib.sha512(data).digest()).decode()
    if got != SHA512:
        print(f"lucide-static {VERSION}'s tarball isn't the one pinned (sha512-{got})", file=sys.stderr)
        return 1

    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
        icons = {
            Path(m.name).stem: m
            for m in tar.getmembers()
            if m.isfile() and m.name.startswith("package/icons/") and m.name.endswith(".svg")
        }
        OUT.mkdir(parents=True, exist_ok=True)
        written = []
        for name in sorted(names):
            m = icons.get(name)
            if m is None:
                print(f"lucide-static has no {name}", file=sys.stderr)
                return 1
            svg = tar.extractfile(m).read().decode()
            (OUT / f"{name}.svg").write_text(tidy(svg) + "\n")
            written.append(name)

    LISTED.write_text(
        f"# lucide-static {VERSION} (ISC license): the icons wad-gtk compiles in\n" + "\n".join(written) + "\n"
    )
    print(f"wrote {len(written)} icons to assets/lucide")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
