#!/usr/bin/env bash
# Builds the images wadspaces run on, here: the builder and the base, and
# packages and sets as wadd puts them together (the same Containerfiles,
# the same labels). Nothing is pushed, and nothing secret goes in.
#
#   images/build.sh                       the builder, then the base
#   images/build.sh --only base           just one (builder or base; base
#                                         still builds the programs in the builder)
#   images/build.sh --no-cache            without podman's layer cache
#   images/build.sh pkg <id> [--needs a,b] [--probe <cmd>] [-- <command>...]
#                                         a package, captured on the base: a
#                                         feature (`wadspaces-feature <id>`),
#                                         rpm-<app> (`wadspaces-dnf`, with
#                                         --desktop f --rpms a,b) or <command>;
#                                         what it needs must be built first
#   images/build.sh set <id>...           a set: those packages (built first
#                                         when they aren't) stacked on the
#                                         base and assembled
#
#   builder  localhost/wados-builder:44       Fedora 44 with Rust 1.99: builds
#            wadspaces-clipboard (crates/wad-clip), wadspaces-session
#            (crates/wad-session) and selkies-desktop (apps/selkies-desktop)
#            into .build/images/bin, with cargo's caches in podman volumes
#   base     localhost/wados-base:44          the desktop every wadspace runs on
#   pkg      localhost/wados-pkg-<id>:latest  (and :<image id 12>) one install's
#            files on their own, FROM scratch (wadspaces-pkg capture)
#   chain    localhost/wados-chain:<sha12>    the base with packages copied on,
#            one per image
#   set      localhost/wados-set-<key>:latest the top chain, assembled
#            (wadspaces-pkg assemble)
#
# CI will publish packages as ghcr.io/hunggod/wados-pkg-<id>:<abi> (the
# base's io.wadspaces.abi label); this script never pushes.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${HERE}/.." && pwd)"
BASE="${WADOS_BASE:-localhost/wados-base:44}"
BUILDER=localhost/wados-builder:44
BIN="${ROOT}/.build/images/bin"
NO_CACHE=()
BUILT_BUILDER=false

usage() { sed -n '2,33p' "$0" | sed 's/^# \{0,1\}//'; }
log() { echo ">> $*"; }

build_builder() {
    ${BUILT_BUILDER} && return
    log "building ${BUILDER} from images/builder/"
    podman build -q "${NO_CACHE[@]}" -t "${BUILDER}" "${HERE}/builder" > /dev/null
    BUILT_BUILDER=true
}

# The programs that run in every wadspace, built for Fedora 44 in the
# builder, with the checkout mounted read-only. Cargo builds in a workspace
# of their own (the root Cargo.toml for its shared tables and [profile.image],
# members cut down to the two crates), so other crates being written can't
# break the image's build, and nothing in the checkout is written. Cargo.lock
# pins their dependencies: the build stops if it needed one Cargo.lock
# hasn't got (as --locked would).
build_bins() {
    build_builder
    log "building wadspaces-clipboard, wadspaces-session and selkies-desktop (in the builder)"
    rm -rf "${BIN}" && mkdir -p "${BIN}"
    # label=disable, not :z: relabelling would rewrite the whole checkout.
    podman run --rm --security-opt label=disable \
        -v "${ROOT}":/src:ro \
        -v wados-cargo-registry:/usr/local/cargo/registry \
        -v wados-target-44:/target -e CARGO_TARGET_DIR=/target \
        -v "${BIN}":/out \
        "${BUILDER}" \
        bash -c 'set -eo pipefail
            rm -rf /tmp/ws && mkdir -p /tmp/ws/crates && cd /tmp/ws
            cp /src/Cargo.toml /src/Cargo.lock . && cp Cargo.lock /tmp/pinned.lock
            cp -r /src/crates/wad-clip /src/crates/wad-session crates/
            sed -i "s|^members = .*|members = [\"crates/wad-clip\", \"crates/wad-session\"]|" Cargo.toml
            cargo build --profile image -p wad-clip -p wad-session
            pairs() { grep -A1 "^name = " "$1" | grep -v "^--" | paste - - | sort -u; }
            extra="$(comm -13 <(pairs /tmp/pinned.lock) <(pairs Cargo.lock))"
            [[ -z "${extra}" ]] || { echo "Cargo.lock does not pin: ${extra}" >&2; exit 1; }
            cp /target/image/wadspaces-clipboard /target/image/wadspaces-session /out/
            cp -r /src/apps/selkies-desktop /tmp/sd && make -s -C /tmp/sd && cp /tmp/sd/selkies-desktop /out/'
}

build_base() {
    build_bins
    log "building ${BASE} from images/base/"
    podman build "${NO_CACHE[@]}" \
        --build-arg BUILD_DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)" --build-arg VERSION="$(date -u +%Y%m%d)" \
        --build-context "bin=${BIN}" \
        -t "${BASE}" "${HERE}/base"
}

need_base() {
    podman image exists "${BASE}" || { echo "${BASE} isn't built: images/build.sh --only base" >&2; exit 1; }
}

image_id() { podman image inspect --format '{{.Id}}' "$1"; }
label_of() { podman image inspect --format "{{index .Labels \"$2\"}}" "$1"; }
json() { python3 -c 'import json, sys; print(json.dumps(sys.argv[1:]) if len(sys.argv) != 2 else json.dumps(sys.argv[1]))' "$@"; }
sha12() { python3 -c 'import hashlib, sys; print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest()[:12])'; }

# A package's ref, by image id (as wadd names them): localhost/wados-pkg-<id>:<id12>.
pkg_ref() {
    local img="localhost/wados-pkg-$1:latest"
    podman image exists "${img}" || return 1
    echo "localhost/wados-pkg-$1:$(image_id "${img}" | cut -c1-12)"
}

# What a package is, as crates/wad-gen's recipes (recipes.rs) say for
# phase 1: a feature (its needs, its probe), or an app's RPMs (rpm-<app>:
# Fedora's packages and the launcher they install).
needs_for() {
    case "$1" in
        rust) echo cpp ;;
    esac
}
probe_for() {
    case "$1" in
        vscode) echo "code --version --no-sandbox --user-data-dir /tmp/wadspaces-probe" ;;
        rust) echo "rustc --version" ;;
        cpp) echo "cmake --version" ;;
        nodejs) echo "node --version" ;;
        python) echo "python3 --version" ;;
    esac
}
rpm_app() {  # rpm-<app> -> "<desktop> <package>..."
    case "$1" in
        rpm-krita) echo "org.kde.krita.desktop krita" ;;
    esac
}

# Packages in WadOS's order (wad-gen's packages::canonical): the features
# in recipes.rs's ORDER, then Fedora's apps, then extensions, each by id.
canonical() {
    python3 -c '
import sys
order = ["cpp", "python", "nodejs", "rust", "vscode"]
def rank(i):
    if i in order: return (0, order.index(i), i)
    return (1 if i.startswith("rpm-") else 2 if i.startswith("ext-") else 3, 0, i)
print("\n".join(sorted(dict.fromkeys(sys.argv[1:]), key=rank)))' "$@"
}

# A package's Containerfile, as wad-gen generates it (packages.rs): the
# base, what it needs copied on and assembled, the capture; then its files
# FROM scratch.
#   pkg <feature>                        wadspaces-feature <feature>
#   pkg rpm-<app> [--desktop f] [--rpms a,b]   wadspaces-dnf <app> … && wadspaces-dnf --clean
#   pkg <id> [--needs a,b] [--probe cmd] -- <command>...
build_pkg() {
    local id="$1"; shift
    local needs probe="" cmd=() desktop="" rpms=""
    needs="$(needs_for "${id}")"
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --needs) needs="$2"; shift 2 ;;
            --probe) probe="$2"; shift 2 ;;
            --desktop) desktop="$2"; shift 2 ;;
            --rpms) rpms="$2"; shift 2 ;;
            --) shift; cmd=("$@"); break ;;
            *) echo "pkg: unknown option $1" >&2; exit 2 ;;
        esac
    done
    [[ "${id}" =~ ^[a-z0-9][a-z0-9.+-]*$ ]] || { echo "bad package id: ${id}" >&2; exit 2; }
    if [[ ${#cmd[@]} -eq 0 && "${id}" == rpm-* ]]; then
        local app="${id#rpm-}" known dnf
        known="$(rpm_app "${id}")"
        [[ -n "${desktop}" ]] || desktop="${known%% *}"
        [[ -n "${rpms}" ]] || rpms="$( [[ -n "${known}" ]] && echo "${known#* }" || echo "${app}")"
        dnf="wadspaces-dnf ${app}${desktop:+ --desktop ${desktop}} ${rpms//,/ }"
        cmd=(sh -c "${dnf} && wadspaces-dnf --clean")
        [[ -n "${probe}" ]] || probe="rpm -q --qf '%{VERSION}-%{RELEASE}' ${rpms%%[ ,]*}"
    fi
    [[ ${#cmd[@]} -gt 0 ]] || cmd=(wadspaces-feature "${id}")
    [[ -n "${probe}" ]] || probe="$(probe_for "${id}")"
    need_base
    local abi ctx file capture=(wadspaces-pkg capture "${id}" --out /out) n r
    abi="$(label_of "${BASE}" io.wadspaces.abi)"
    ctx="$(mktemp -d)"
    file="${ctx}/Containerfile"
    {
        echo "# The ${id} package: what ${cmd[*]} adds to the base. Generated by images/build.sh."
        echo "ARG BASE_IMAGE=${BASE}"
        echo 'FROM ${BASE_IMAGE} AS build'
        if [[ -n "${needs}" ]]; then
            for n in ${needs//,/ }; do
                r="$(pkg_ref "${n}")" || { echo "${id} needs ${n} first: images/build.sh pkg ${n}" >&2; exit 1; }
                echo "COPY --from=${r} / /"
            done
            echo "RUN wadspaces-pkg assemble"
            capture+=(--needs "${needs}")
        fi
        [[ -n "${probe}" ]] && capture+=(--probe "${probe}")
        capture+=(-- "${cmd[@]}")
        echo "RUN $(json "${capture[@]}")"
        echo
        echo "FROM scratch"
        echo "COPY --from=build /out/ /"
        echo "LABEL io.wadspaces.pkg=$(json "${id}") io.wadspaces.pkg.needs=$(json "${needs}")"
    } > "${file}"
    log "capturing localhost/wados-pkg-${id} on ${BASE}: ${cmd[*]}"
    podman build "${NO_CACHE[@]}" -t "localhost/wados-pkg-${id}:capture" -f "${file}" "${ctx}" || exit 1
    # What the capture said about it (its meta.json), as labels, the way CI
    # labels what it publishes.
    local ctr meta
    ctr="$(podman create -q "localhost/wados-pkg-${id}:capture" /none)"
    meta="$(podman cp "${ctr}:/usr/share/wadspaces/pkg/${id}/meta.json" - | tar -xO)"
    podman rm -f "${ctr}" > /dev/null
    printf 'FROM localhost/wados-pkg-%s:capture\nLABEL io.wadspaces.pkg.version=%s io.wadspaces.pkg.abi=%s io.wadspaces.pkg.content=%s io.wadspaces.pkg.built-here="true"\n' \
        "${id}" "$(json "$(jq -r .version <<< "${meta}")")" "$(json "${abi}")" "$(json "$(jq -r .content <<< "${meta}")")" \
        > "${file}"
    podman build -q -t "localhost/wados-pkg-${id}:latest" -f "${file}" "${ctx}" > /dev/null
    podman rmi -f "localhost/wados-pkg-${id}:capture" > /dev/null 2>&1 || true
    rm -rf "${ctx}"
    r="localhost/wados-pkg-${id}:$(image_id "localhost/wados-pkg-${id}:latest" | cut -c1-12)"
    podman tag "localhost/wados-pkg-${id}:latest" "${r}"
    log "built ${r}: $(jq -c '{version, files, bytes, rpms: (.rpms | length), triggers}' <<< "${meta}")"
}

# A set, as wad-gen builds one (sets.rs): its packages and what they need,
# in WadOS's order, each copied onto the chain under it (one image each,
# named by the base and the packages so far), then assembled. Its key is
# wad-gen's set_key with no commands of its own.
build_set() {
    need_base
    local ids=() all=() id n
    for id in "$@"; do
        for n in $(needs_for "${id}" | tr ',' ' '); do all+=("${n}"); done
        all+=("${id}")
    done
    mapfile -t ids < <(canonical "${all[@]}")
    for id in "${ids[@]}"; do
        pkg_ref "${id}" > /dev/null || build_pkg "${id}"
    done
    local key base_id from from_id chain=() pairs="" ref ctag ctx
    base_id="$(image_id "${BASE}")"
    key="$(python3 -c 'import json, sys; print(json.dumps(["wados-set", 1, sys.argv[1], sys.argv[2:], []], separators=(",", ":")))' \
        "${base_id#sha256:}" "${ids[@]}" | tr -d '\n' | sha12)"
    from="${BASE}" from_id="${base_id}"
    ctx="$(mktemp -d)"
    for id in "${ids[@]}"; do
        ref="$(pkg_ref "${id}")"
        chain+=("${ref}")
        pairs+="${pairs:+,}${id}@${ref##*:}"
        ctag="localhost/wados-chain:$( { printf '%s' "${base_id#sha256:}"; printf '|%s' "${chain[@]}"; } | sha12)"
        if ! podman image exists "${ctag}" || [[ "$(label_of "${ctag}" io.wadspaces.from)" != "${from_id}" ]]; then
            log "stacking ${id} (${ctag})"
            printf '# A chain image. Generated by images/build.sh.\nFROM %s\nCOPY --from=%s / /\nLABEL io.wadspaces.chain=%s io.wadspaces.from=%s\n' \
                "${from}" "${ref}" "$(json "${pairs}")" "$(json "${from_id}")" > "${ctx}/Containerfile"
            podman build -q -t "${ctag}" "${ctx}" > /dev/null || exit 1
        fi
        from="${ctag}" from_id="$(image_id "${ctag}")"
    done
    log "putting ${ids[*]} together (wadspaces-pkg assemble): localhost/wados-set-${key}:latest"
    {
        echo "# A wadspace's set: its packages put together. Generated by images/build.sh."
        echo "ARG BASE_IMAGE=${from}"
        echo 'FROM ${BASE_IMAGE}'
        echo "RUN wadspaces-pkg assemble"
        echo "LABEL io.wadspaces.layer=$(json "set:${key}") io.wadspaces.dockerfile-sha256=$(json "${key}")" \
            "io.wadspaces.from=$(json "${base_id}") io.wadspaces.pkgs=$(json "${pairs}") io.wadspaces.assembly=\"1\""
    } > "${ctx}/Containerfile"
    podman build "${NO_CACHE[@]}" -t "localhost/wados-set-${key}:latest" "${ctx}" || exit 1
    rm -rf "${ctx}"
    echo "localhost/wados-set-${key}:latest"
}

ONLY=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --only) [[ $# -gt 1 && " builder base " == *" $2 "* ]] || { echo "--only builder|base" >&2; exit 2; }
                ONLY+=("$2"); shift 2 ;;
        --no-cache) NO_CACHE=(--no-cache); shift ;;
        pkg) shift; [[ $# -ge 1 ]] || { usage >&2; exit 2; }; build_pkg "$@"; exit 0 ;;
        set) shift; [[ $# -ge 1 ]] || { usage >&2; exit 2; }; build_set "$@"; exit 0 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done
[[ ${#ONLY[@]} -gt 0 ]] || ONLY=(builder base)
for name in "${ONLY[@]}"; do
    case "${name}" in
        builder) build_builder ;;
        base) build_base ;;
    esac
done
log "built: ${ONLY[*]}"
