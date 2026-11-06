# tools/vm.sh update-check: WadOS's own update on a real VM (phase 8), as
# wadd does it (apps/wadd/src/updates.rs), with real bootc: found, staged,
# turned down and staged again, restarted into, then rolled back. Sourced by
# tools/vm.sh, whose helpers it uses. One VM (VM_NAME update:
# .build/vm/update/), from a fresh layer over wados.qcow2, thrown away after.
# Every step says ok or FAIL, and the check goes on after a FAIL where it can.
#
#   tools/vm.sh update-check
#
# wados.qcow2 must be made from the image the update is built on
# (UPDATE_FROM, localhost/wados-host:latest: tools/vm.sh disk after
# host/build.sh image), so the VM runs this wadd.
#
#   1. the update: localhost/wados-host:next, FROM UPDATE_FROM with a marker
#      file (/usr/share/wados/update-marker) and a newer version label; a
#      registry on the laptop (127.0.0.1, 10.0.2.2 from the VM) with it as
#      wados-host:latest
#   2. a fresh VM; bootc follows that registry (`bootc switch
#      --mutate-in-place`: what it follows, without a download; the
#      registry's insecure, for the test, in registries.conf.d); bootc's own
#      timer is masked
#   3. GET /v1/update: booted, nothing available or staged; POST check: an
#      Update job (its stage), which finds it and downloads it (the apply
#      job, chained: Downloading, staging); staged is the update's version
#   4. Don't apply it (POST cancel): nothing staged, and it's available
#      again; Download (POST apply): staged again
#   5. restarted: it boots the update (the marker's there, bootc says so);
#      wadd says booted is it, and the one before is kept to roll back to
#   6. Roll back (POST rollback): the one before is staged; restarted: it
#      boots that (no marker); wadd agrees
#   7. no registry login was written (/run/ostree/auth.json: the image's public)
#   Then: the VM, its layer, the registry and :next go.

U_NAME=update
U_FROM="${UPDATE_FROM:-localhost/wados-host:latest}"
U_NEXT=localhost/wados-host:next
U_REG=wados-vm-update-registry
U_MARKER=/usr/share/wados/update-marker

ok() { printf 'ok    %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; FAILED=$((FAILED + 1)); }
note() { printf '      %s\n' "$*"; }
indent() { sed 's/^/      | /'; }

u_port() { cat "${VM}/${U_NAME}/ssh.port"; }
as_root() { ssh_to "$(u_port)" root "$@"; }

# wadd's API through its socket, as root: sets CODE and BODY; true on a 2xx.
call() {
    local m="$1" p="$2" out
    out="$(as_root "curl -sS --max-time 120 --unix-socket /run/wadd/wadd.sock -X ${m} -w '\n%{http_code}' \
        -H 'content-type: application/json' $([[ "${m}" == POST ]] && echo "-d '{}'") http://wadd${p}" < /dev/null 2>&1)"
    CODE="${out##*$'\n'}"
    BODY="${out%$'\n'*}"
    [[ "${BODY}" == "${CODE}" ]] && BODY=""
    [[ "${CODE}" == 2?? ]]
}
get() { call GET "$1" && printf '%s' "${BODY}"; }
field() { jq -r "$1 // empty" <<< "$2" 2> /dev/null; }

# Waits for a job to end: its JSON; true if it's done.
job_done() {
    local id="$1" secs="${2:-600}" st deadline=$(($(now) + ${2:-600}))
    while (($(now) < deadline)); do
        if call GET "/v1/jobs/${id}"; then
            st="$(jq -r '.job.status // .status' <<< "${BODY}" 2> /dev/null)"
            case "${st}" in
                done) printf '%s' "${BODY}"; return 0 ;;
                error | cancelled) printf '%s' "${BODY}"; return 1 ;;
            esac
        fi
        sleep 3
    done
    printf 'still %s after %d s' "${st:-?}" "${secs}"
    return 1
}
stages_of() { jq -r '(.job // .) | .stages[]? | "\(.label): \(.state)\(if .error then " (" + .error + ")" else "" end)"' <<< "$1" 2> /dev/null; }

# Until `cond` (a jq filter on GET /v1/update) holds, `secs` at most: the status.
update_until() {
    local cond="$1" secs="${2:-600}" u deadline=$(($(now) + ${2:-600}))
    while (($(now) < deadline)); do
        u="$(get /v1/update)"
        [[ -n "${u}" && "$(jq -r "${cond}" <<< "${u}" 2> /dev/null)" == true ]] && { printf '%s' "${u}"; return 0; }
        sleep 3
    done
    printf '%s' "${u}"
    return 1
}

u_boot() {
    local t
    t=$(now)
    boot --fresh > /dev/null
    if wait_ssh "$(u_port)" root 420 "${RUN}/qemu.pid"; then
        ok "root's SSH after $(($(now) - t)) s"
    else
        fail "no SSH within 420 s (${RUN}/serial.log)"
        tail -n 20 "${RUN}/serial.log" | indent
        return 1
    fi
}

# Restarts the VM (as the person's Restart does: systemd reboots it) and waits for it.
u_restart() {
    local t old
    t=$(now)
    old="$(as_root 'cat /proc/sys/kernel/random/boot_id' 2> /dev/null)"
    as_root 'systemctl reboot' > /dev/null 2>&1 || true
    sleep 10
    local deadline=$(($(now) + 480)) id=""
    while (($(now) < deadline)); do
        running "${RUN}/qemu.pid" || { fail "the VM went away while restarting"; return 1; }
        id="$(as_root 'cat /proc/sys/kernel/random/boot_id' 2> /dev/null)"
        [[ -n "${id}" && "${id}" != "${old}" ]] && break
        sleep 5
    done
    if [[ -n "${id}" && "${id}" != "${old}" ]]; then
        ok "restarted: SSH again after $(($(now) - t)) s"
    else
        fail "it didn't come back within 480 s"
        return 1
    fi
    # wadd answers.
    local i
    for i in $(seq 40); do call GET /v1/health && break; sleep 3; done
}

u_cleanup() {
    set +e
    [[ -n "${U_KEEP:-}" ]] || stop > /dev/null 2>&1
    rm -f "${RUN}/run.qcow2" "${RUN}/vars.fd"
    podman rm -f -t 2 "${U_REG}" > /dev/null 2>&1
    podman rmi "${U_NEXT}" > /dev/null 2>&1
}

update_check() {
    set +e
    FAILED=0
    VM_NAME="${U_NAME}" RUN="${VM}/${U_NAME}"
    local t0 next_v from_v port u job j staged
    t0=$(now)
    trap 'u_cleanup' EXIT
    trap 'exit 130' INT TERM
    [[ -f "${VM}/wados.qcow2" ]] || { fail "no ${VM}/wados.qcow2: tools/vm.sh disk"; return 1; }
    local d
    for d in "${VM}" "${VM}"/*/; do
        d="${d%/}"
        [[ "${d##*/}" == "${U_NAME}" ]] && continue
        running "${d}/qemu.pid" && { fail "another VM is running (${d##*/}): one VM at a time"; return 1; }
    done
    running "${VM}/builder.pid" && { fail "the builder VM is running: one VM at a time"; return 1; }
    note "the laptop: $(free -m | awk '/^Mem:/ { print $7 " MB available" }') for ${MEM} MB"

    say "1. the update: ${U_NEXT}, on ${U_FROM}, in a registry here"
    podman image exists "${U_FROM}" || { fail "no ${U_FROM}: host/build.sh image"; return 1; }
    from_v="$(podman image inspect --format '{{index .Labels "org.opencontainers.image.version"}}' "${U_FROM}")"
    # Newer: the same day, a later build (or a later day, for an image without one).
    next_v="$(python3 -I -c 'import sys, time
v = sys.argv[1].split(".")
print(f"{v[0]}.{v[1]}.{int(v[2]) + 1}" if len(v) == 3 and all(p.isdigit() for p in v) else "44." + time.strftime("%Y%m%d") + ".9999")' "${from_v}")"
    if printf 'FROM %s\nRUN mkdir -p %s && echo "%s" > %s\nLABEL org.opencontainers.image.version=%s\n' \
        "${U_FROM}" "$(dirname "${U_MARKER}")" "${next_v}" "${U_MARKER}" "${next_v}" \
        | podman build -q -t "${U_NEXT}" -f - > /dev/null 2>&1; then
        ok "${U_NEXT}: ${next_v} (on ${from_v:-?}), with ${U_MARKER}"
    else
        fail "building ${U_NEXT}"
        return 1
    fi
    podman rm -f -t 2 "${U_REG}" > /dev/null 2>&1
    port="$(free_port)"
    podman run -d --rm --name "${U_REG}" -p "127.0.0.1:${port}:5000" docker.io/library/registry:2 > /dev/null
    for _ in $(seq 50); do curl -sf "http://127.0.0.1:${port}/v2/" > /dev/null && break; sleep 0.2; done
    if podman push -q --tls-verify=false --compression-format zstd --compression-level 1 "${U_NEXT}" \
        "localhost:${port}/wados-host:latest" > /dev/null 2>&1; then
        ok "in the registry: 10.0.2.2:${port}/wados-host:latest (from the VM)"
    else
        fail "pushing ${U_NEXT}"
        return 1
    fi

    say "2. a fresh VM, following that registry"
    stop > /dev/null 2>&1
    rm -f "${RUN}/run.qcow2" "${RUN}/vars.fd"
    u_boot || return 1
    local follows="10.0.2.2:${port}/wados-host:latest"
    if as_root bash -s -- "${port}" "${follows}" << 'EOF' 2>&1 | indent; then
set -e
cat > /etc/containers/registries.conf.d/50-wados-vm-update.conf << CONF
# tools/vm.sh update-check: the laptop's throwaway registry (the test's, not the image's).
[[registry]]
location = "10.0.2.2:$1"
insecure = true
CONF
# /sysroot is read-only: writable for this, in a namespace of its own.
unshare -m sh -c "mount -o remount,rw /sysroot && bootc switch --mutate-in-place --transport registry $2"
EOF
        ok "bootc follows ${follows}"
    else
        fail "bootc switch --mutate-in-place"
    fi
    [[ "$(as_root 'systemctl is-enabled bootc-fetch-apply-updates.timer' 2> /dev/null)" == masked ]] \
        && ok "bootc's own timer is masked (wadd checks; the person restarts)" \
        || fail "bootc-fetch-apply-updates.timer isn't masked"

    say "3. wadd finds it and stages it"
    for _ in $(seq 40); do call GET /v1/health && break; sleep 3; done
    u="$(get /v1/update)"
    if [[ "$(field .booted.version "${u}")" == "${from_v}" && -z "$(field .staged "${u}")" ]]; then
        ok "GET /v1/update: booted ${from_v}, online $(field .online "${u}"), nothing staged"
    else
        fail "GET /v1/update: ${u:-${CODE}}"
    fi
    if call POST /v1/update/check; then
        job="$(field '.job.id // .id' "${BODY}")"
        ok "POST /v1/update/check: the Update job ${job}"
        j="$(job_done "${job}" 300)" && ok "the check: done" || fail "the check: $(field '.job.error // .error' "${j}")"
        stages_of "${j}" | indent
    else
        fail "POST /v1/update/check: ${CODE} ${BODY}"
    fi
    if u="$(update_until '.staged != null and .job == null' 900)"; then
        staged="$(field .staged.version "${u}")"
        [[ "${staged}" == "${next_v}" ]] && ok "found and downloaded by itself: ${staged} is staged" \
            || fail "staged ${staged}, not ${next_v}"
        j="$(call GET /v1/jobs && jq -c '[.[] | (.job // .) | select(.kind == "update")] | .[0]' <<< "${BODY}")"
        note "the download: $(jq -r '"\(.title): \(.status), \(.totalBytes) bytes"' <<< "${j}" 2> /dev/null)"
        stages_of "${j}" | indent
    else
        fail "nothing staged within 900 s: ${u}"
    fi
    as_root 'bootc status --format humanreadable' 2>&1 | head -20 | indent

    say "4. Don't apply it, then Download"
    if call POST /v1/update/cancel; then
        [[ -z "$(field .staged "${BODY}")" && "$(field .available.version "${BODY}")" == "${next_v}" ]] \
            && ok "POST /v1/update/cancel: nothing staged; ${next_v} available again" \
            || fail "after cancel: ${BODY}"
        [[ "$(as_root "bootc status --json | jq -r '.status.staged'" 2> /dev/null)" == null ]] \
            && ok "bootc says nothing's staged" || fail "bootc still has one staged"
    else
        fail "POST /v1/update/cancel: ${CODE} ${BODY}"
    fi
    if call POST /v1/update/apply; then
        job="$(field '.job.id // .id' "${BODY}")"
        j="$(job_done "${job}" 900)" && ok "POST /v1/update/apply: done" || fail "apply: $(field '.job.error // .error' "${j}")"
        u="$(get /v1/update)"
        [[ "$(field .staged.version "${u}")" == "${next_v}" ]] && ok "${next_v} staged again" || fail "after apply: ${u}"
    else
        fail "POST /v1/update/apply: ${CODE} ${BODY}"
    fi

    say "5. restarted into it"
    u_restart || return 1
    [[ "$(as_root "cat ${U_MARKER}" 2> /dev/null)" == "${next_v}" ]] && ok "the marker's there: it booted ${next_v}" \
        || fail "no ${U_MARKER}: it didn't boot the update"
    u="$(get /v1/update)"
    if [[ "$(field .booted.version "${u}")" == "${next_v}" && "$(field .rollback.version "${u}")" == "${from_v}" ]]; then
        ok "wadd: booted ${next_v}, ${from_v} kept to roll back to"
    else
        fail "wadd after the restart: ${u}"
    fi

    say "6. rolled back"
    if call POST /v1/update/rollback; then
        [[ "$(field .staged.version "${BODY}")" == "${from_v}" ]] && ok "POST /v1/update/rollback: ${from_v} is staged" \
            || fail "after rollback: ${BODY}"
    else
        fail "POST /v1/update/rollback: ${CODE} ${BODY}"
    fi
    u_restart || return 1
    as_root "test -e ${U_MARKER}" 2> /dev/null && fail "the marker's still there" || ok "no marker: it booted ${from_v} again"
    u="$(get /v1/update)"
    [[ "$(field .booted.version "${u}")" == "${from_v}" ]] && ok "wadd: booted ${from_v} ($(field .rollback.version "${u}") to roll back to)" \
        || fail "wadd after rolling back: ${u}"

    say "7. no login was written"
    as_root "test -e /run/ostree/auth.json" 2> /dev/null && fail "/run/ostree/auth.json is there" \
        || ok "no /run/ostree/auth.json: the image is public"
    as_root "journalctl -b -u wadd --no-pager | grep -i 'updates' | tail -5" 2> /dev/null | indent

    echo
    if ((FAILED == 0)); then
        echo "update-check: all ok ($(($(now) - t0)) s)"
    else
        echo "update-check: ${FAILED} FAILED ($(($(now) - t0)) s)"
    fi
    return "${FAILED}"
}
