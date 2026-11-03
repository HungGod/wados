#!/bin/bash
# Builds labwc with WadOS's patches (host/labwc/*.patch), for development:
# .build/labwc/labwc, which tools/headless.sh runs. It's built in a fedora:44
# container against Fedora's own wlroots, so it runs on a Fedora 44 laptop
# beside the labwc it has. host/Containerfile builds the machine's the same way.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
version=0.9.6
out=$root/.build/labwc
mkdir -p "$out"
if [ ! -d "$out/src" ]; then
  git clone -q --depth 1 --branch "$version" https://github.com/labwc/labwc.git "$out/src"
  for p in "$root"/host/labwc/*.patch; do git -C "$out/src" apply "$p"; done
fi
podman run --rm -v "$out/src":/src:Z registry.fedoraproject.org/fedora:44 bash -c '
  set -e
  dnf -y -q install dnf5-plugins >/dev/null
  dnf -y -q builddep labwc >/dev/null
  cd /src
  [ -d build ] || meson setup build --buildtype=release -Dwerror=false >/dev/null
  ninja -C build >/dev/null'
cp "$out/src/build/labwc" "$out/labwc"
echo "$out/labwc"
