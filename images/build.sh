#!/usr/bin/env bash
# Builds the images wadspaces run on, here: the builder, the base and the
# stream sidecar, and packages and sets as wadd puts them together. What a
# package or a set is (what it needs, WadOS's order, its Containerfile, its
# labels and its key) is wad-gen's, asked through wados-pkgs
# (crates/wad-gen/src/bin/wados-pkgs), so this script, wadd and CI build the
# same images. Nothing is pushed, and nothing secret goes in.
#
#   images/build.sh                       the builder, the base, then the stream
#   images/build.sh --only base           just one (builder, base or stream; base
#                                         still builds the programs in the builder)
#   images/build.sh --no-cache            without podman's layer cache
#   images/build.sh pkg <id>              a package, captured on the base: one
#                                         wad-gen has a recipe for (a feature,
#                                         rpm-<app> or ext-<extension>; to try
#                                         a new one, add it to
#                                         crates/wad-gen/src/recipes.rs); what
#                                         it needs must be built first
#   images/build.sh set <id>...           a set: those packages and what they
#                                         need (built first when they aren't)
#                                         stacked on the base and assembled
#
#   builder  localhost/wados-builder:44       Fedora 44 with Rust 1.99: builds
#            wadbrowser (apps/wadbrowser), wadspaces-clipboard
#            (crates/wad-clip), wadspaces-session (crates/wad-session) and
#            selkies-desktop (apps/selkies-desktop) into .build/images/bin
#            (WadBrowser's launcher and icon into .build/images/share), with
#            cargo's caches in podman volumes
#   base     localhost/wados-base:44          the desktop every wadspace runs on
#   stream   localhost/wados-stream:44        the stream sidecar: upstream Selkies
#            2.2.1 (images/stream/requirements.txt, pinned by sha256) on
#            Fedora 44, beside a wadspace while it's streamed
#   pkg      localhost/wados-pkg-<id>:latest  (and :<image id 12>) one install's
#            files on their own, FROM scratch (wadspaces-pkg capture)
#   chain    localhost/wados-chain:<sha12>    the base with packages copied on,
#            one per image
#   set      localhost/wados-set-<key>:latest the top chain, assembled
#            (wadspaces-pkg assemble)
#
# pkg and set need cargo (wados-pkgs is built from the checkout) and jq.
# WADOS_PKGS names a wados-pkgs already built (CI's); WADOS_BASE another base.
# CI (.github/workflows/) publishes the builder, the base and the stream as
# ghcr.io/hunggod/wados-{builder,base,stream}:44 and packages as
# ghcr.io/hunggod/wados-pkg-<id>:<abi> (the base's io.wadspaces.abi label);
# this script never pushes.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${HERE}/.." && pwd)"
BASE="${WADOS_BASE:-localhost/wados-base:44}"
BUILDER=localhost/wados-builder:44
STREAM=localhost/wados-stream:44
BIN="${ROOT}/.build/images/bin"
SHARE="${ROOT}/.build/images/share"
NO_CACHE=()
BUILT_BUILDER=false
PKGS="${WADOS_PKGS:-}"

usage() { awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; }
log() { echo ">> $*"; }

build_builder() {
    ${BUILT_BUILDER} && return
    log "building ${BUILDER} from images/builder/"
    podman build -q "${NO_CACHE[@]}" -t "${BUILDER}" "${HERE}/builder" > /dev/null
    BUILT_BUILDER=true
}

# The programs that run in every wadspace, built for Fedora 44 in the
# builder, with the checkout mounted read-only. Cargo builds in a workspace
# of their own: the root Cargo.toml (for its shared tables and
# [profile.image]) with its members cut down to the three programs, the
# crates they use and their tests' (wad-wadd-client's are wadd's) coming
# along as path dependencies, and assets/ (WadBrowser's pages embed some).
# So other programs being written can't break the image's build, and
# nothing in the checkout is written. Cargo.lock pins their dependencies:
# the build stops if it needed one Cargo.lock hasn't got (as --locked
# would). WadBrowser's launcher and icon go to ${SHARE}, the base's `share`
# build context (/usr/share).
build_bins() {
    build_builder
    log "building wadbrowser, wadspaces-clipboard, wadspaces-session and selkies-desktop (in the builder)"
    rm -rf "${BIN}" "${SHARE}" && mkdir -p "${BIN}" "${SHARE}"
    # label=disable, not :z: relabelling would rewrite the whole checkout.
    podman run --rm --security-opt label=disable \
        -v "${ROOT}":/src:ro \
        -v wados-cargo-registry:/usr/local/cargo/registry \
        -v wados-target-44:/target -e CARGO_TARGET_DIR=/target \
        -v "${BIN}":/out \
        "${BUILDER}" \
        bash -c 'set -eo pipefail
            rm -rf /tmp/ws && mkdir -p /tmp/ws && cd /tmp/ws
            cp /src/Cargo.toml /src/Cargo.lock . && cp Cargo.lock /tmp/pinned.lock
            cp -r /src/crates /src/apps /src/assets .
            sed -i "s|^members = .*|members = [\"crates/wad-clip\", \"crates/wad-session\", \"apps/wadbrowser\"]|" Cargo.toml
            cargo build --profile image -p wad-clip -p wad-session -p wadbrowser
            pairs() { grep -A1 "^name = " "$1" | grep -v "^--" | paste - - | sort -u; }
            extra="$(comm -13 <(pairs /tmp/pinned.lock) <(pairs Cargo.lock))"
            [[ -z "${extra}" ]] || { echo "Cargo.lock does not pin: ${extra}" >&2; exit 1; }
            cp /target/image/wadbrowser /target/image/wadspaces-clipboard /target/image/wadspaces-session /out/
            cp -r /src/apps/selkies-desktop /tmp/sd && make -s -C /tmp/sd && cp /tmp/sd/selkies-desktop /out/'
    local data="${ROOT}/apps/wadbrowser/data"
    install -Dm644 "${data}/io.wados.Browser.desktop" "${SHARE}/applications/io.wados.Browser.desktop"
    install -Dm644 "${data}/icons/io.wados.Browser.svg" "${SHARE}/icons/hicolor/scalable/apps/io.wados.Browser.svg"
}

build_base() {
    build_bins
    log "building ${BASE} from images/base/"
    podman build "${NO_CACHE[@]}" \
        --build-arg BUILD_DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)" --build-arg VERSION="$(date -u +%Y%m%d)" \
        --build-context "bin=${BIN}" --build-context "share=${SHARE}" \
        -t "${BASE}" "${HERE}/base"
}

# The stream sidecar: no programs of ours, just Selkies' pinned wheels
# (images/stream/lock.sh writes them) on linuxserver's Fedora 44 base.
build_stream() {
    log "building ${STREAM} from images/stream/"
    podman build "${NO_CACHE[@]}" \
        --build-arg BUILD_DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)" --build-arg VERSION="$(date -u +%Y%m%d)" \
        -t "${STREAM}" "${HERE}/stream"
}

need_base() {
    podman image exists "${BASE}" || { echo "${BASE} isn't built: images/build.sh --only base" >&2; exit 1; }
}

# wados-pkgs, for pkg and set: WADOS_PKGS, or built from the checkout (a
# debug build: quick to build, and quick enough for this).
find_pkgs() {
    [[ -n "${PKGS}" ]] && return
    command -v jq > /dev/null || { echo "pkg and set need jq" >&2; exit 1; }
    log "building wados-pkgs (cargo build -p wad-gen --bin wados-pkgs)"
    PKGS="$(cd "${ROOT}" && cargo build -q -p wad-gen --bin wados-pkgs --message-format=json-render-diagnostics |
        jq -r 'select(.executable != null) | .executable' | tail -n1)"
    [[ -x "${PKGS}" ]] || { echo "couldn't build wados-pkgs" >&2; exit 1; }
}
# wad-gen's answer (wados-pkgs prints it; it says why when it won't).
pkgs() { "${PKGS}" "$@"; }

image_id() { podman image inspect --format '{{.Id}}' "$1"; }
label_of() { podman image inspect --format "{{index .Labels \"$2\"}}" "$1"; }

# A package's ref, by image id (wad-gen's local_ref, as wadd names them):
# localhost/wados-pkg-<id>:<id12>. :latest is this script's name for the
# newest one (wadd tags the ones it builds so too).
pkg_ref() {
    local img="localhost/wados-pkg-$1:latest" id
    podman image exists "${img}" || return 1
    id="$(image_id "${img}")"
    pkgs local-ref "$1" "${id}"
}

# A package, as wadd and CI build one: wad-gen's Containerfile (the base,
# what it needs copied on and assembled, the capture; then its files FROM
# scratch), then labelled with what the capture says (its meta.json), as CI
# labels what it publishes, and as built here.
build_pkg() {
    [[ $# -eq 1 && "$1" != -* ]] || { echo "pkg takes one package id: wad-gen's recipe for it is the whole recipe" >&2; exit 2; }
    local id="$1" list info abi n r ctx file ctr
    need_base
    list="$(pkgs packages "${id}")"
    info="$(jq -c --arg id "${id}" '.[] | select(.id == $id)' <<< "${list}")"
    local needs=() args=()
    mapfile -t needs < <(jq -r '.needs[]' <<< "${info}")
    for n in "${needs[@]}"; do
        r="$(pkg_ref "${n}")" || { echo "${id} needs ${n} first: images/build.sh pkg ${n}" >&2; exit 1; }
        args+=(--need "${n}=${r}")
    done
    abi="$(label_of "${BASE}" io.wadspaces.abi)"
    ctx="$(mktemp -d)"
    file="${ctx}/Containerfile"
    pkgs containerfile "${id}" --base "${BASE}" "${args[@]}" > "${file}"
    log "capturing localhost/wados-pkg-${id} ($(jq -r .name <<< "${info}")) on ${BASE}"
    podman build "${NO_CACHE[@]}" -t "localhost/wados-pkg-${id}:capture" -f "${file}" "${ctx}" || exit 1
    ctr="$(podman create -q "localhost/wados-pkg-${id}:capture" /none)"
    podman cp "${ctr}:$(jq -r .meta <<< "${info}")" - | tar -xO > "${ctx}/meta.json"
    podman rm -f "${ctr}" > /dev/null
    {
        echo "FROM localhost/wados-pkg-${id}:capture"
        pkgs labels "${id}" --meta "${ctx}/meta.json" --abi "${abi}" --built-here
    } > "${file}"
    podman build -q -t "localhost/wados-pkg-${id}:latest" -f "${file}" "${ctx}" > /dev/null
    podman rmi -f "localhost/wados-pkg-${id}:capture" > /dev/null 2>&1 || true
    r="$(pkg_ref "${id}")"
    podman tag "localhost/wados-pkg-${id}:latest" "${r}"
    log "built ${r}: $(jq -c '{version, files, bytes, rpms: (.rpms | length), triggers}' "${ctx}/meta.json")"
    rm -rf "${ctx}"
}

# A set, as wadd builds one: its packages and what they need, in WadOS's
# order, each copied onto the chain under it (one image each, named by the
# base and the packages so far), then assembled. Its key is wad-gen's
# set_key with no commands of its own. Prints the set's image, last.
build_set() {
    need_base
    local base_id set key image id ref ids=() chain=()
    base_id="$(image_id "${BASE}")"
    set="$(pkgs set "${base_id}" "$@")"
    mapfile -t ids < <(jq -r '.packages[]' <<< "${set}")
    key="$(jq -r .key <<< "${set}")"
    image="$(jq -r .image <<< "${set}")"
    # (Not `pkg_ref || build_pkg`: errexit is off in the right of an ||, in
    # all build_pkg does.)
    for id in "${ids[@]}"; do
        if ! pkg_ref "${id}" > /dev/null; then build_pkg "${id}"; fi
    done
    local from="${BASE}" from_id="${base_id}" ctag ctx
    ctx="$(mktemp -d)"
    for id in "${ids[@]}"; do
        ref="$(pkg_ref "${id}")"
        chain+=("${id}=${ref}")
        ctag="$(pkgs chain-ref "${base_id}" "${chain[@]}")"
        if ! podman image exists "${ctag}" || [[ "$(label_of "${ctag}" io.wadspaces.from)" != "${from_id}" ]]; then
            log "stacking ${id} (${ctag})"
            pkgs chain-step "${from}" "${from_id}" "${chain[@]}" > "${ctx}/Containerfile"
            podman build -q -t "${ctag}" "${ctx}" > /dev/null || exit 1
        fi
        from="${ctag}"
        from_id="$(image_id "${ctag}")"
    done
    log "putting ${ids[*]} together (wadspaces-pkg assemble): ${image}"
    pkgs set-containerfile "${key}" "${from}" "${base_id}" "${chain[@]}" > "${ctx}/Containerfile"
    podman build "${NO_CACHE[@]}" -t "${image}" "${ctx}" || exit 1
    rm -rf "${ctx}"
    echo "${image}"
}

ONLY=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --only) [[ $# -gt 1 && " builder base stream " == *" $2 "* ]] || { echo "--only builder|base|stream" >&2; exit 2; }
                ONLY+=("$2"); shift 2 ;;
        --no-cache) NO_CACHE=(--no-cache); shift ;;
        pkg) shift; [[ $# -ge 1 ]] || { usage >&2; exit 2; }; find_pkgs; build_pkg "$@"; exit 0 ;;
        set) shift; [[ $# -ge 1 ]] || { usage >&2; exit 2; }; find_pkgs; build_set "$@"; exit 0 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done
[[ ${#ONLY[@]} -gt 0 ]] || ONLY=(builder base stream)
for name in "${ONLY[@]}"; do
    case "${name}" in
        builder) build_builder ;;
        base) build_base ;;
        stream) build_stream ;;
    esac
done
log "built: ${ONLY[*]}"
