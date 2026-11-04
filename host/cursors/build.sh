#!/bin/bash
# Renders the WadOS cursor theme (host/cursors/*.svg) into an Xcursor theme
# at $1 (host/Containerfile's cursors stage: rsvg-convert, xcursorgen). Each
# cursor at 24, 32, 48, 64 and 96 px (XCURSOR_SIZE 24 at 100%-400%), its
# hotspot scaled from the 32 px drawing's; the names apps ask for link to it.
# Every other cursor is Adwaita's (index.theme: Inherits).
set -euo pipefail
src="$(cd "$(dirname "$0")" && pwd)"
out="$1"
work="$(mktemp -d)"
mkdir -p "${out}/cursors"
render() {
    local name="$1" hx="$2" hy="$3" s
    shift 3
    : > "${work}/${name}.cfg"
    for s in 24 32 48 64 96; do
        rsvg-convert -w "${s}" -h "${s}" "${src}/${name}.svg" -o "${work}/${name}-${s}.png"
        echo "${s} $(( (hx * s + 16) / 32 )) $(( (hy * s + 16) / 32 )) ${work}/${name}-${s}.png" >> "${work}/${name}.cfg"
    done
    xcursorgen "${work}/${name}.cfg" "${out}/cursors/${name}"
    for alias in "$@"; do ln -sf "${name}" "${out}/cursors/${alias}"; done
}
render default 4 3 left_ptr arrow top_left_arrow
render pointer 13 3 hand2 hand1 pointing_hand
render text 16 16 xterm ibeam
install -m 644 "${src}/index.theme" "${out}/index.theme"
rm -rf "${work}"
