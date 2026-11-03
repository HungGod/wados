#!/usr/bin/env bash
# Proves the images work: the base built, two packages captured apart on it
# (rpm-krita: Fedora's krita through wadspaces-dnf, as wad-gen installs RPM
# apps; and the nodejs feature), a set assembling both, and that set behaving as a
# Fedora system and as a wadspace's desktop. Each check prints ok or FAIL;
# the script exits 1 if any failed.
#
#   images/check.sh              build what's missing (podman's cache keeps the rest)
#   images/check.sh --no-build   only check images built already
#   images/check.sh --no-display skip the desktop check
#
# The desktop check runs the set as wadd would on the machine's labwc: a
# headless patched labwc (tools/headless.sh, tools/build-labwc.sh), a socket
# it serves to one sandboxed client (crates/wad-wayland's listen example,
# app id io.wadspaces.check) mounted at /run/wadspaces-display, abc mapped to
# you (PUID 1000), and the window listed with its app id (the toplevels
# example). A screenshot goes to .build/images/check-desktop.png.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "${HERE}/.." && pwd)"
BASE=localhost/wados-base:44
PKGS=(rpm-krita nodejs)
BUILD=true
DISPLAY_CHECK=true
for a in "$@"; do
    case "$a" in
        --no-build) BUILD=false ;;
        --no-display) DISPLAY_CHECK=false ;;
        *) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
    esac
done

fails=0
check() { if eval "$2"; then echo "ok    $1"; else echo "FAIL  $1"; fails=$((fails + 1)); fi; }
in_img() { local img="$1"; shift; podman run --rm --entrypoint '' "${img}" "$@"; }

if ${BUILD}; then
    "${HERE}/build.sh" --only base || exit 1
    for id in "${PKGS[@]}"; do "${HERE}/build.sh" pkg "${id}" || exit 1; done
fi
SET="$("${HERE}/build.sh" set "${PKGS[@]}" | tail -n1)" || exit 1
[[ "${SET}" == localhost/wados-set-* ]] || { echo "no set built (${SET})"; exit 1; }
echo "== ${SET}"

# ------------------------------------------------------------- the packages
for id in "${PKGS[@]}"; do
    img="localhost/wados-pkg-${id}:latest"
    rpms="$(podman create -q "${img}" /none)"
    list="$(podman cp "${rpms}:/usr/share/wadspaces/pkg/${id}/rpms" - 2> /dev/null | tar -xO)"
    podman rm -f "${rpms}" > /dev/null
    n="$(grep -c . <<< "${list}")"
    echo "== ${id}: $(podman image inspect --format '{{index .Labels "io.wadspaces.pkg.version"}}' "${img}"), ${n} rpms," \
        "$(podman image inspect --format '{{.Size}}' "${img}" | numfmt --to=si)B"
    check "${id}'s package has its RPM headers (rpmdb, rpms)" "[[ ${n} -gt 0 ]]"
    check "${id}'s package has no rpm database of its own" \
        "! podman export \$(podman create -q ${img} /none) | tar -t | grep -q '^usr/lib/sysimage/rpm/'"
    podman ps -aq --filter ancestor="${img}" | xargs -r podman rm -f > /dev/null
done

# ------------------------------------------------------------- the set as Fedora
# Every RPM either package brought is installed, rpm -V passes on all of
# them, assembling again changes nothing, and dnf works on top as a
# wadspace's own RUN would.
report="$(in_img "${SET}" bash -c '
    cat /usr/share/wadspaces/pkg/*/rpms | sort -u > /tmp/want
    rpm -qa --qf "%{NEVRA}\n" | sort -u > /tmp/have
    echo "want $(wc -l < /tmp/want) missing $(comm -23 /tmp/want /tmp/have | wc -l) total $(wc -l < /tmp/have)"
    rpm -q krita > /dev/null && rpm -q --whatprovides nodejs > /dev/null && echo "named ok"
    rpm -V $(sed "s/^gpg-pubkey.*//" /tmp/want) > /tmp/verify 2>&1; echo "verify $? $(wc -l < /tmp/verify)"
    sed "s/^/  rpm -V: /" /tmp/verify | head -20
    before="$(rpm -qa | sort | sha256sum)"
    wadspaces-pkg assemble | tail -n1
    [ "$(rpm -qa | sort | sha256sum)" = "${before}" ] && echo "again ok"
    rpmdb --verifydb && echo "verifydb ok"
    echo "dnf: $(dnf -q repoquery --installed --qf "%{name}\n" krita "nodejs*" 2> /dev/null | sort | tr "\n" " ")"
')"
echo "${report}" | sed 's/^/   /'
check "rpm -qa lists every RPM rpm-krita's and nodejs's packages brought" "grep -qE '^want [1-9][0-9]* missing 0 ' <<< \"\${report}\""
check "rpm -q krita, rpm -q --whatprovides nodejs" "grep -qx 'named ok' <<< \"\${report}\""
check "rpm -V passes on all of them" "grep -qx 'verify 0 0' <<< \"\${report}\""
check "assembling again changes nothing" "grep -qx 'again ok' <<< \"\${report}\""
check "rpm's database verifies" "grep -qx 'verifydb ok' <<< \"\${report}\""
check "dnf sees them installed" "grep -qE '^dnf: .*krita .*nodejs' <<< \"\${report}\""

# A wadspace's own build step on top of the set.
ctx="$(mktemp -d)"
printf 'FROM %s\nRUN dnf -y install tree && dnf clean all\n' "${SET}" > "${ctx}/Containerfile"
podman build -q -t localhost/wados-check-run:latest "${ctx}" > "${ctx}/log" 2>&1
check "dnf install -y tree on top of the set (a wadspace's RUN)" "podman image exists localhost/wados-check-run:latest"
after="$(in_img localhost/wados-check-run:latest bash -c 'tree --version | head -n1; rpm -q krita tree > /dev/null && rpm -q --whatprovides nodejs > /dev/null && echo still; rpm -V tree && echo verified' 2>&1)"
echo "${after}" | sed 's/^/   /'
check "tree runs, rpm -V passes, krita and nodejs still installed" "grep -qx still <<< \"\${after}\" && grep -qx verified <<< \"\${after}\""
podman rmi -f localhost/wados-check-run:latest > /dev/null 2>&1
rm -rf "${ctx}"

# ------------------------------------------------------------- the desktop
if ${DISPLAY_CHECK}; then
    ex="${ROOT}/target/debug/examples"
    if [[ ! -x "${ex}/listen" || ! -x "${ex}/toplevels" ]]; then
        (cd "${ROOT}" && cargo build -q -p wad-wayland --examples) || exit 1
    fi
    [[ -x "${ROOT}/.build/labwc/labwc" ]] || "${ROOT}/tools/build-labwc.sh" > /dev/null || exit 1
    # shellcheck source=../tools/headless.sh
    . "${ROOT}/tools/headless.sh"
    ctr=wados-check-desktop
    lpid=""
    cleanup() {
        podman rm -f -t 3 "${ctr}" > /dev/null 2>&1
        [[ -n "${lpid}" ]] && kill "${lpid}" 2> /dev/null
        stop_labwc
    }
    trap cleanup EXIT
    start_labwc 1 || exit 1
    # The wadspace's own folder, as wadd makes it: one socket in it.
    mkdir -p "${hl_rt}/ws-check"
    "${ex}/listen" "${hl_rt}/wayland-0" "${hl_rt}/ws-check/wayland-1" io.wadspaces.check > /dev/null 2>&1 &
    lpid=$!
    for _ in $(seq 50); do [[ -S "${hl_rt}/ws-check/wayland-1" ]] && break; sleep 0.1; done
    # Rootless, as wadd runs a wadspace: abc is uid 1000 inside and you
    # outside (the socket is yours), every other id a subordinate one.
    maps=(--uidmap 0:1:1000 --uidmap 1000:0:1 --uidmap 1001:1001:64535
          --gidmap 0:1:1000 --gidmap 1000:0:1 --gidmap 1001:1001:64535)
    podman run -d --rm --name "${ctr}" --security-opt label=disable "${maps[@]}" \
        -e PUID=1000 -e PGID=1000 \
        -v "${hl_rt}/ws-check":/run/wadspaces-display:ro \
        "${SET}" > /dev/null
    tops=""
    for _ in $(seq 60); do
        tops="$(in_labwc "${ex}/toplevels" 2> /dev/null)"
        grep -q 'app_id="io.wadspaces.check"' <<< "${tops}" && break
        sleep 1
    done
    echo "${tops}" | sed 's/^/   toplevels: /'
    check "the wadspace's labwc drew a window on the machine's labwc" "grep -q 'title=\"labwc' <<< \"\${tops}\""
    check "its window reports app_id io.wadspaces.check" "grep -q 'app_id=\"io.wadspaces.check\"' <<< \"\${tops}\""
    sleep 3
    mkdir -p "${ROOT}/.build/images"
    shot HEADLESS-1 "${ROOT}/.build/images/check-desktop.png" 2> /dev/null && echo "   screenshot: .build/images/check-desktop.png"
    logs="$(podman logs "${ctr}" 2>&1)"
    grep -E '^\[(svc-de|desktop-init|wadspaces-workspace)\]' <<< "${logs}" | sed 's/^/   /'
    inside="$(podman exec "${ctr}" sh -c 'pgrep -a labwc; pgrep -a selkies-desktop; pgrep -a swaybg' 2>&1)"
    echo "${inside}" | sed 's/^/   ps: /'
    check "selkies-desktop runs in it" "grep -q selkies-desktop <<< \"\${inside}\""
    [[ ${fails} -gt 0 ]] && echo "${logs}" | tail -30
fi

[[ ${fails} -eq 0 ]] && echo "all ok" || echo "${fails} failed"
exit $((fails > 0))
