#!/usr/bin/env bash
# Builds the WadOS host image (the machine's own OS), and disk images of it.
#
#   host/build.sh image              the image: localhost/wados-host:latest
#                                    (--variant nvidia: localhost/wados-host-nvidia:latest)
#   host/build.sh bins               just WadOS's binaries, into .build/host/bin
#   host/build.sh qcow2 | iso | raw  the image, then a disk image of it in output/
#                                    (bootc-image-builder, with sudo), with a test
#                                    user for a VM
#   host/build.sh push               the images built, to ghcr.io/hunggod/wados-host[-nvidia]
#                                    (:latest and a dated tag). Log in first:
#                                    podman login ghcr.io
#   host/build.sh installer /dev/sdX | --image FILE
#                                    the installer stick (sudo): the image installed
#                                    on it, with the secrets folder's cloudflare.toml
#                                    on it and nowhere else. Without that file,
#                                    machines it installs have no public address
#
#   --variant host|nvidia[,...]      which image (host unless told); qcow2, iso
#                                    and raw take one
#   --test-key                       nvidia: sign NVIDIA's modules with a throwaway
#                                    key made in .build/mok-test/ (to try the
#                                    signing path); that image is tagged
#                                    :test-key and is never pushed
#
# The variants:
#   host    the open graphics drivers (Intel, AMD, nouveau/NVK): the Surface,
#           a new machine, and the installer stick run it (host/Containerfile)
#   nvidia  host plus NVIDIA's own driver, its kernel modules built here for
#           the image's kernel and signed with the MOK key (host/Containerfile.nvidia)
#
# What `image` does: the binaries (cargo in localhost/wados-host-builder:44,
# host/builder; caches in .build/host), then the image (labwc with
# host/labwc/*.patch is built in its first stage), then rechunked by package
# with rpm-ostree (RECHUNK=0: as built, quicker for trying things).
#
# Secrets. host/secrets is gitignored, kept out of the image's context by
# .containerignore, and hidden from the cargo builder (which mounts the whole
# repo) by a tmpfs over it: `notmpcopyup`, for by default podman copies what's
# under a tmpfs into it. None is in this repo, the build's context or a layer: the image is
# the same for everyone. The one this script uses is NVIDIA's module signing
# key, from ${SECRETS}/mok/: host/secrets, or WADOS_SECRETS (wadspaces.priv and
# wadspaces.der, or one .priv and one .der): it's handed to podman as build
# secrets, which only the stage that signs the modules sees. Without it the
# nvidia variant is skipped. The stick's `installer` reads cloudflare.toml
# from the same folder, onto the stick only. Nothing else here reads that folder.
#
# The image's version (org.opencontainers.image.version, which bootc and the
# Updates page show): 44.<the day, UTC>.<hour and minute>, or WADOS_VERSION.
#
# Env: JOBS (cargo's jobs, 2), RECHUNK (1), WADOS_SECRETS, WADOS_VERSION,
#      WADOS_VM_PASSWORD, WADOS_VM_SSH_KEY (a public key, or a file of one): qcow2|iso|raw's test user.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${HERE}/.." && pwd)"
BUILD="${ROOT}/.build/host"
IMAGE_BASE=localhost/wados-host
BUILDER=localhost/wados-host-builder:44
REGISTRY=ghcr.io/hunggod
# The base every image here is built and rechunked with (host/Containerfile's).
BOOTC=quay.io/fedora/fedora-bootc:44@sha256:291bda73006a5cfc6d4ac24f9a261915624f9f555d9ff1a426cb412928f152ac
# host/secrets (gitignored), unless WADOS_SECRETS says, or it isn't there.
SECRETS="${WADOS_SECRETS:-}"
if [[ -z "${SECRETS}" ]]; then
    SECRETS="${HERE}/secrets"
    [[ -d "${SECRETS}" ]] || SECRETS="${HOME}/.config/wados/secrets"
fi
JOBS="${JOBS:-2}"
VERSION="${WADOS_VERSION:-44.$(date -u +%Y%m%d).$((10#$(date -u +%H%M)))}"

log() { echo ">> $*"; }
die() { echo "host/build.sh: $*" >&2; exit "${2:-1}"; }
usage() { sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'; }

TARGET="${1:-}"
[[ -n "${TARGET}" ]] || { usage; exit 2; }
shift
VARIANTS=host
TEST_KEY=false
STICK_ARGS=()
if [[ "${TARGET}" == installer ]]; then
    STICK_ARGS=("$@")
    set --
fi
while [[ $# -gt 0 ]]; do
    case "$1" in
        --variant) VARIANTS="${2:?--variant takes host or nvidia}"; shift 2 ;;
        --test-key) TEST_KEY=true; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option: $1" 2 ;;
    esac
done
VARIANTS="${VARIANTS//,/ }"
for v in ${VARIANTS}; do
    [[ "${v}" == host || "${v}" == nvidia ]] || die "unknown variant: ${v} (host or nvidia)" 2
done
wants() { [[ " ${VARIANTS} " == *" $1 "* ]]; }

# The image a variant is tagged as. A test-key build of nvidia isn't
# :latest, so nothing mistakes it for one to push or install.
image_of() {
    case "$1" in
        host) echo "${IMAGE_BASE}:latest" ;;
        nvidia) if ${TEST_KEY}; then echo "${IMAGE_BASE}-nvidia:test-key"; else echo "${IMAGE_BASE}-nvidia:latest"; fi ;;
    esac
}

# ------------------------------------------------------------ the binaries
# wadd, the shell, wadsctl, the clipboard bridge, WadBrowser, Settings and Files, release
# builds for Fedora 44, built in the host's builder with the checkout
# read-only (label disabled: relabelling would rewrite the whole checkout).
# Cargo's registry and target dir are kept under .build/host, so a rebuild
# only builds what changed. Cargo.lock must already pin everything
# (--locked). Stripped into .build/host/bin; their symbols are kept in
# .build/host/debug. WadBrowser's launcher and icon, and Settings' and Files'
# launchers, go to .build/host/share,
# the image's `share` context (/usr/share).
build_bins() {
    log "building ${BUILDER} (host/builder)"
    podman build -q -t "${BUILDER}" "${HERE}/builder" > /dev/null
    mkdir -p "${BUILD}/cargo/registry" "${BUILD}/cargo/git" "${BUILD}/target"
    rm -rf "${BUILD}/bin" "${BUILD}/debug" "${BUILD}/share" && mkdir -p "${BUILD}/bin" "${BUILD}/debug"
    log "building wadd, wados-shell, wadsctl, wadspaces-clipboard, wadbrowser, wados-settings, wados-files, wados-installer and wados-agents (cargo -j${JOBS}, in the builder)"
    podman run --rm --security-opt label=disable \
        -v "${ROOT}:/src:ro" \
        --tmpfs /src/host/secrets:notmpcopyup \
        -v "${BUILD}/cargo/registry:/usr/local/cargo/registry" \
        -v "${BUILD}/cargo/git:/usr/local/cargo/git" \
        -v "${BUILD}/target:/target" -e CARGO_TARGET_DIR=/target \
        -v "${BUILD}/bin:/out" -v "${BUILD}/debug:/debug" \
        -w /src "${BUILDER}" \
        bash -c 'set -eo pipefail
            cargo build --release --locked -j "$0" -p wadd -p wados-shell -p wadsctl -p wad-clip -p wadbrowser \
                -p wados-settings -p wados-files -p wados-installer -p wados-agents
            for b in wadd wados-shell wadsctl wadspaces-clipboard wadbrowser wados-settings wados-files wados-installer wados-agents; do
                objcopy --only-keep-debug "/target/release/${b}" "/debug/${b}.debug"
                install -s -m 755 "/target/release/${b}" "/out/${b}"
            done' "${JOBS}"
    ls -l "${BUILD}/bin" | tail -n +2 | awk '{ printf ">>   %-22s %6.1f MB\n", $9, $5 / 1048576 }'
    local data="${ROOT}/apps/wadbrowser/data"
    install -Dm644 "${data}/io.wados.Browser.desktop" "${BUILD}/share/applications/io.wados.Browser.desktop"
    install -Dm644 "${data}/icons/io.wados.Browser.svg" "${BUILD}/share/icons/hicolor/scalable/apps/io.wados.Browser.svg"
    for app in Settings Files Agents; do
        install -Dm644 "${ROOT}/apps/wados-${app,,}/data/io.wados.${app}.desktop" \
            "${BUILD}/share/applications/io.wados.${app}.desktop"
    done
}

# ------------------------------------------------------------ the MOK key
MOK_KEY=""
MOK_DER=""

# The key that signs NVIDIA's modules: the secrets folder's, or with
# --test-key a throwaway one. Only file names are looked at here; podman
# reads the files, as build secrets. Returns 1 when there's none.
find_mok() {
    if ${TEST_KEY}; then
        local dir="${ROOT}/.build/mok-test"
        if [[ ! -s "${dir}/test.priv" || ! -s "${dir}/test.der" ]]; then
            (umask 077 && mkdir -p "${dir}" && openssl req -new -x509 -newkey rsa:2048 -nodes -days 30 \
                -subj "/CN=WadOS TEST module key (never push)/" \
                -addext "keyUsage=digitalSignature" -addext "extendedKeyUsage=codeSigning" \
                -keyout "${dir}/test.priv" -outform DER -out "${dir}/test.der" 2> /dev/null)
        fi
        echo
        echo "  !!  TEST KEY: NVIDIA's modules are signed with a throwaway key"
        echo "  !!  (.build/mok-test/). The image, $(image_of nvidia), is only for"
        echo "  !!  trying the signing path: never push it, never install it."
        echo
        MOK_KEY="${dir}/test.priv"
        MOK_DER="${dir}/test.der"
        return 0
    fi
    local dir="${SECRETS}/mok"
    if [[ -f "${dir}/wadspaces.priv" && -f "${dir}/wadspaces.der" ]]; then
        MOK_KEY="${dir}/wadspaces.priv"
        MOK_DER="${dir}/wadspaces.der"
        return 0
    fi
    local privs=() ders=()
    shopt -s nullglob
    privs=("${dir}"/*.priv)
    ders=("${dir}"/*.der)
    shopt -u nullglob
    if [[ ${#privs[@]} -eq 1 && ${#ders[@]} -eq 1 ]]; then
        MOK_KEY="${privs[0]}"
        MOK_DER="${ders[0]}"
        return 0
    fi
    return 1
}

# ------------------------------------------------------------ images
# The built image $1 (from its :unchunked build) in chunks by package
# (rpm-ostree), WadOS's own files in a chunk of their own (their
# user.component xattr): an update downloads only the chunks that changed.
# Rootless: the image is mounted into a fedora-bootc container that writes
# an OCI directory, copied into podman's storage with its layers
# uncompressed, so their digests are the same in every build that didn't
# change them.
rechunk() {
    local raw="$1" image="$2" out="${ROOT}/.build/chunked"
    if [[ "${RECHUNK:-1}" == "0" ]]; then
        podman tag "${raw}" "${image}"
        return
    fi
    log "rechunking ${image} (a few minutes)"
    rm -rf "${out}" && mkdir -p "${out}/oci/blobs/sha256"
    echo '{"imageLayoutVersion":"1.0.0"}' > "${out}/oci/oci-layout"
    echo '{"schemaVersion":2,"manifests":[]}' > "${out}/oci/index.json"
    # A rootfs carries no labels: the built image's go on again (--bootc
    # sets bootc's own).
    local labels=() l
    while IFS= read -r l; do
        [[ -n "${l}" && "${l}" != containers.bootc=* && "${l}" != ostree.bootable=* ]] && labels+=(--label "${l}")
    done < <(podman image inspect --format '{{range $k, $v := .Labels}}{{$k}}={{$v}}{{"\n"}}{{end}}' "${raw}")
    podman run --rm --privileged --security-opt label=disable \
        --mount "type=image,source=${raw},destination=/rootfs" -v "${out}:/out" \
        "${BOOTC}" \
        rpm-ostree compose build-chunked-oci --bootc --format-version=1 --max-layers 64 \
        "${labels[@]}" --rootfs /rootfs --output oci:/out/oci:host > "${out}.log" 2>&1 \
        || { tail -20 "${out}.log" >&2; die "rechunking ${image} failed (${out}.log)"; }
    skopeo copy -q --dest-decompress "oci:${out}/oci:host" "dir:${out}/dir"
    skopeo copy -q "dir:${out}/dir" "containers-storage:${image}"
    rm -rf "${out}"
    log "${image}: $(podman image inspect --format '{{len .RootFS.Layers}} layers, {{.Size}} bytes' "${image}")"
}

# The images asked for, one at a time (this machine hasn't the memory for
# two): host first, since nvidia is built on it, then each rechunked.
build_images() {
    build_bins
    log "building ${IMAGE_BASE}:unchunked (host/Containerfile)"
    podman build --jobs 1 -f "${HERE}/Containerfile" \
        --build-context "bin=${BUILD}/bin" \
        --build-context "share=${BUILD}/share" \
        --build-context "assets=${ROOT}/assets" \
        --label "org.opencontainers.image.version=${VERSION}" \
        -t "${IMAGE_BASE}:unchunked" "${HERE}"
    log "${IMAGE_BASE}: version ${VERSION}"
    local built=(host)
    if wants nvidia; then
        if find_mok; then
            log "building ${IMAGE_BASE}-nvidia (host/Containerfile.nvidia: NVIDIA's modules are compiled, a while)"
            # The key and its certificate as build secrets: in /run/secrets
            # for the signing step only, never in a layer.
            podman build --jobs 1 -f "${HERE}/Containerfile.nvidia" \
                --build-arg "BASE=${IMAGE_BASE}:unchunked" \
                --build-arg "MOK=$(${TEST_KEY} && echo test || echo wados)" \
                --secret "id=mok_key,src=${MOK_KEY}" \
                --secret "id=mok_der,src=${MOK_DER}" \
                -t "${IMAGE_BASE}-nvidia:unchunked" "${HERE}"
            built+=(nvidia)
        else
            echo ">> SKIPPING the nvidia variant: no MOK key in ${SECRETS}/mok/"
            echo ">>   (wadspaces.priv and wadspaces.der, or one .priv and one .der)."
            echo ">>   NVIDIA's modules are signed with it; this script never makes one."
            echo ">>   To try the signing path: host/build.sh image --variant nvidia --test-key"
        fi
    fi
    local v
    for v in "${built[@]}"; do
        wants "${v}" || continue
        local raw="${IMAGE_BASE}:unchunked"
        [[ "${v}" == nvidia ]] && raw="${IMAGE_BASE}-nvidia:unchunked"
        rechunk "${raw}" "$(image_of "${v}")"
    done
}

# ------------------------------------------------------------ disk images
# bootc-image-builder's settings for a VM, in .build (never committed):
# a test user, uid 1000 like the machine's person, in wados (wadd's socket)
# and wheel, with a password (WADOS_VM_PASSWORD, else made up and printed)
# and an SSH key (WADOS_VM_SSH_KEY, else none).
bib_config() {
    local out="$1" password="${WADOS_VM_PASSWORD:-}" key="${WADOS_VM_SSH_KEY:-}" hash
    if [[ -z "${password}" ]]; then
        password="$(python3 -c 'import secrets; print(secrets.token_urlsafe(9))')"
        echo ">> the test user: sam, password ${password}"
    fi
    hash="$(printf '%s' "${password}" | openssl passwd -6 -stdin)"
    if [[ -n "${key}" && -f "${key}" ]]; then key="$(< "${key}")"; fi
    mkdir -p "$(dirname "${out}")"
    (umask 077 && python3 - "${hash}" "${key}" > "${out}" <<'EOF'
import json, sys
hash, key = sys.argv[1], sys.argv[2].strip()
print("# bootc-image-builder's settings for a test VM (host/build.sh): generated, never committed.")
print("[[customizations.user]]")
print('name = "sam"')
print('description = "Sam"')
print("uid = 1000")
print("password = " + json.dumps(hash))
if key:
    print("key = " + json.dumps(key))
print('groups = ["wheel", "wados"]')
print()
print("# The wadspaces' images are large (a few GB each): room for them.")
print("[[customizations.filesystem]]")
print('mountpoint = "/"')
print('minsize = "60 GiB"')
EOF
    )
}

disk_image() {
    local image; image="$(image_of "${VARIANTS}")"
    [[ "${image}" != *:test-key ]] || die "a test-key image isn't for a disk"
    podman image exists "${image}" || die "${image} isn't built: host/build.sh image${VARIANTS:+ --variant ${VARIANTS}}"
    local bib="${ROOT}/.build/bib/config.toml"
    bib_config "${bib}"
    mkdir -p "${ROOT}/output"
    log "sudo is needed for bootc-image-builder"
    sudo -v
    # bootc-image-builder reads the image from root's storage.
    log "copying ${image} into root's container storage"
    podman image scp "${USER}@localhost::${image}" root@localhost::
    sudo podman run --rm -it --privileged --pull=newer \
        --security-opt label=type:unconfined_t \
        -v "${ROOT}/output:/output" \
        -v /var/lib/containers/storage:/var/lib/containers/storage \
        -v "${bib}:/config.toml:ro" \
        quay.io/centos-bootc/bootc-image-builder:latest \
        --type "${TARGET}" --rootfs xfs --config /config.toml "${image}"
    log "disk image in ${ROOT}/output/"
}

# ------------------------------------------------------------ push
# Every image is checked before anything is sent: built, and not signed
# with a test key (--test-key's images are refused by name and by label).
push() {
    ${TEST_KEY} && die "a test-key image is never pushed" 2
    local tag v src dest
    for v in ${VARIANTS}; do
        src="$(image_of "${v}")"
        podman image exists "${src}" || die "${src} isn't built"
        [[ "$(podman image inspect --format '{{index .Labels "io.wados.mok"}}' "${src}")" != test ]] \
            || die "${src} is signed with a test key: never pushed" 2
    done
    skopeo login --get-login ghcr.io > /dev/null 2>&1 || die "not logged in to ghcr.io: podman login ghcr.io" 2
    tag="$(date +%Y%m%d)-$(git -C "${ROOT}" rev-parse --short HEAD 2> /dev/null || echo nogit)"
    git -C "${ROOT}" diff --quiet HEAD -- 2> /dev/null || tag="${tag}-dirty"
    for v in ${VARIANTS}; do
        src="$(image_of "${v}")"
        dest="${REGISTRY}/$(basename "${src%:*}")"
        log "pushing ${src} to ${dest}:latest and :${tag}"
        skopeo copy --retry-times 3 "containers-storage:${src}" "docker://${dest}:latest"
        skopeo copy -q --retry-times 3 "docker://${dest}:latest" "docker://${dest}:${tag}"
    done
}

# ------------------------------------------------------------ the stick
# The image installed on the stick with bootc (a stick is a machine that
# boots to WadSpaces Live), then host/stick-files. No ISO: an install copies
# the stick's own system to the disk.
installer() {
    local where="${1:-}" file="${2:-}" image="${IMAGE_BASE}:latest" dev
    local target="ghcr.io/hunggod/wados-host:latest"
    podman image exists "${image}" || die "${image} isn't built: host/build.sh image"
    case "${where}" in
        --image) [[ -n "${file}" ]] || die "--image FILE" 2 ;;
        /dev/*) [[ -b "${where}" ]] || die "${where} isn't a disk" 2 ;;
        *) die "installer /dev/sdX | --image FILE" 2 ;;
    esac
    local keys="${SECRETS}/cloudflare.toml"
    if [[ -f "${keys}" ]]; then
        log "machines will get a public address: ${keys} goes on the stick"
    else
        log "no ${keys}: machines this installs have no public address"
        keys=""
    fi
    sudo -v
    log "copying ${image} into root's container storage"
    podman image scp "${USER}@localhost::${image}" root@localhost::
    local bootc=(sudo podman run --rm --privileged --pid=host --security-opt label=type:unconfined_t
        -v /dev:/dev -v /var/lib/containers:/var/lib/containers)
    local args=(--wipe --generic-image --skip-fetch-check --target-imgref "${target}")
    if [[ "${where}" == --image ]]; then
        truncate -s 32G "${file}"
        file="$(realpath "${file}")"
        dev="$(sudo losetup -fP --show "${file}")"
        trap 'sudo losetup -d "${dev}" 2> /dev/null || true' EXIT
    else
        dev="${where}"
        log "${dev} is erased: $(lsblk -ndo SIZE,MODEL "${dev}")"
    fi
    "${bootc[@]}" "${image}" bootc install to-disk "${args[@]}" "${dev}"
    sudo udevadm settle
    local mnt root var
    mnt="$(mktemp -d)"
    root="$(lsblk -nrpo NAME,PARTLABEL,LABEL "${dev}" | awk '$2 == "root" || $3 == "root" { print $1; exit }')"
    [[ -n "${root}" ]] || die "no root partition on ${dev}"
    sudo mount "${root}" "${mnt}"
    var="$(sudo find "${mnt}/ostree/deploy" -mindepth 2 -maxdepth 2 -name var -type d | head -1)"
    sudo "${HERE}/stick-files" "${var}" "${target}" "${keys}"
    sudo umount "${mnt}"
    rmdir "${mnt}"
    sync
    log "the stick is ready"
}

case "${TARGET}" in
    bins) build_bins ;;
    image)
        start=${SECONDS}
        build_images
        for v in ${VARIANTS}; do
            podman image exists "$(image_of "${v}")" && log "built $(image_of "${v}")"
        done
        log "took $(( (SECONDS - start) / 60 )) min $(( (SECONDS - start) % 60 )) s"
        ;;
    qcow2|iso|raw)
        [[ "${VARIANTS}" != *" "* ]] || die "${TARGET} takes one --variant" 2
        disk_image
        ;;
    push) push ;;
    installer) installer "${STICK_ARGS[@]}" ;;
    *) usage; exit 2 ;;
esac
