#!/usr/bin/env bash
# A test tailnet on the laptop (phase 6), never the real one: Headscale in a
# rootless podman pod, with TLS from a throwaway CA, its embedded DERP, and a
# policy that lets sam's machines use Taildrive. The VMs reach it at
# https://hs.test:8443 (hs.test is 10.0.2.2 in them, the laptop's loopback
# through QEMU's user-mode network); a third node, "the device", runs in the
# same pod. Nothing here touches the laptop's own Tailscale.
#
#   tools/tailnet.sh start         the CA and cert, the config and policy, Headscale up;
#                                  users sam and eve
#   tools/tailnet.sh stop          everything removed: the pod, its volume, the CA and keys
#   tools/tailnet.sh status
#   tools/tailnet.sh hs ARGS…      the headscale CLI, in its container
#   tools/tailnet.sh user NAME     makes a user (if new); prints its numeric id
#   tools/tailnet.sh key USER [--tags tag:x] [--ephemeral]
#                                  a pre-auth key for USER (reusable, 1 h); prints it
#   tools/tailnet.sh approve AUTH-ID USER
#                                  registers a node waiting at <url>/register/<AUTH-ID>
#   tools/tailnet.sh ca            the CA's path (for the VMs' trust stores)
#   tools/tailnet.sh device start  the device: a userspace tailscaled in the pod
#                                  (tailscale 1.104.5, the host image's), not logged in
#   tools/tailnet.sh device ts ARGS…
#                                  its tailscale CLI (its own socket)
#   tools/tailnet.sh device curl ARGS…
#                                  curl through its SOCKS5 proxy, to the tailnet
#   tools/tailnet.sh device stop
#   tools/tailnet.sh socks         the device's SOCKS5 proxy as the laptop reaches it
#                                  (127.0.0.1:TAILNET_SOCKS_PORT): a browser on the
#                                  laptop onto the tailnet, as the device (phase 7)
#   tools/tailnet.sh https on|off  sam's machines get the "https" node attribute, or
#                                  not (the policy reloaded): `tailscale serve --https`
#                                  then configures, though Headscale has no
#                                  certificates to give it (phase 7's private streams)
#
# Files (made per run, never committed): .build/tailnet/ (TAILNET_DIR).
# The CA's key never leaves it; stop deletes it.
#
# **Why it's shaped so.** Both VMs sit behind QEMU's user-mode NAT (each is
# 10.0.2.15), so they can't reach each other directly: everything is relayed
# through the DERP server, over TCP and TLS only (stock clients refuse plain
# DERP). Headscale's embedded DERP is named after server_url's host and port,
# so server_url is https, with a cert for hs.test, 10.0.2.2 and 127.0.0.1.
# The device runs in Headscale's pod, where hs.test is 127.0.0.1 (the pod's
# /etc/hosts), so the laptop's own names aren't changed.
#
# TAILNET_PORT (8443): the port on the laptop's 127.0.0.1, in server_url, and
# Headscale's own (the DERP's port is server_url's, so they're the same).
# TAILNET_SOCKS_PORT (11055): the device's SOCKS5 proxy, published on the
# laptop's 127.0.0.1.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIR="${TAILNET_DIR:-${ROOT}/.build/tailnet}"
# Headscale 0.29.7 (2026-10-30), pinned by its manifest list's digest.
IMAGE=docker.io/headscale/headscale:0.29.7@sha256:11f81665e842809db9fe3006a38c56a66e968baa5abd4bac498e86445572b970
# The device's tailscaled: the host image's own (the VMs' version).
DEVICE_IMAGE="${TAILNET_DEVICE_IMAGE:-localhost/wados-host:latest}"
POD=wados-tailnet
HS=wados-headscale
DEVICE=wados-tailnet-device
VOLUME=wados-tailnet-data
HOST=hs.test
PORT="${TAILNET_PORT:-8443}"
STUN=3478
URL="https://${HOST}:${PORT}"
BASE_DOMAIN=ts.test
DEVICE_SOCK=/run/ts-device/tailscaled.sock
SOCKS=127.0.0.1:1055
SOCKS_PUBLISHED="${TAILNET_SOCKS_PORT:-11055}"

die() { echo "tailnet.sh: $*" >&2; exit 1; }

hs() { podman exec "${HS}" headscale "$@"; }
up() { podman container exists "${HS}" && [[ "$(podman inspect -f '{{.State.Running}}' "${HS}")" == true ]]; }
need_up() { up || die "Headscale isn't running: tools/tailnet.sh start"; }

# ------------------------------------------------------------ the CA

certs() {
    install -d -m 0700 "${DIR}" "${DIR}/etc"
    local d="${DIR}"
    # The CA: P-256, two days, for this run only.
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 2 \
        -subj "/CN=WadOS test tailnet CA (throwaway)" \
        -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign,cRLSign \
        -keyout "${d}/ca.key" -out "${d}/ca.crt" 2> /dev/null
    openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj "/CN=${HOST}" \
        -keyout "${d}/etc/tls.key" -out "${d}/server.csr" 2> /dev/null
    printf '%s\n' "subjectAltName=DNS:${HOST},IP:10.0.2.2,IP:127.0.0.1" \
        "basicConstraints=critical,CA:FALSE" "keyUsage=critical,digitalSignature" \
        "extendedKeyUsage=serverAuth" > "${d}/server.ext"
    openssl x509 -req -in "${d}/server.csr" -CA "${d}/ca.crt" -CAkey "${d}/ca.key" -CAcreateserial \
        -days 2 -extfile "${d}/server.ext" -out "${d}/etc/tls.crt" 2> /dev/null
    rm -f "${d}/server.csr" "${d}/server.ext" "${d}/ca.srl"
    chmod 0600 "${d}/ca.key" "${d}/etc/tls.key"
    openssl verify -CAfile "${d}/ca.crt" "${d}/etc/tls.crt" > /dev/null
}

config() {
    cat > "${DIR}/etc/config.yaml" << EOF
# tools/tailnet.sh's Headscale: the test tailnet (made per run).
server_url: ${URL}
listen_addr: 0.0.0.0:${PORT}
metrics_listen_addr: 127.0.0.1:9090
grpc_listen_addr: 127.0.0.1:50443
grpc_allow_insecure: false
tls_cert_path: /etc/headscale/tls.crt
tls_key_path: /etc/headscale/tls.key
noise:
  private_key_path: /var/lib/headscale/noise_private.key
prefixes:
  v4: 100.64.0.0/10
  v6: fd7a:115c:a1e0::/48
  allocation: sequential
# Its own DERP and nothing else: no Tailscale servers. It's named after
# server_url (hs.test:${PORT}), over TLS.
derp:
  server:
    enabled: true
    region_id: 999
    region_code: wados-test
    region_name: WadOS test DERP
    verify_clients: true
    stun_listen_addr: 0.0.0.0:${STUN}
    private_key_path: /var/lib/headscale/derp_server_private.key
    automatically_add_embedded_derp_region: true
  urls: []
  paths: []
  auto_update_enabled: false
  update_frequency: 24h
disable_check_updates: true
node:
  ephemeral:
    inactivity_timeout: 30m
database:
  type: sqlite
  sqlite:
    path: /var/lib/headscale/db.sqlite
    write_ahead_log: true
log:
  level: info
  format: text
policy:
  mode: file
  path: /etc/headscale/policy.hujson
dns:
  magic_dns: true
  base_domain: ${BASE_DOMAIN}
  override_local_dns: false
  nameservers:
    global: []
  search_domains: []
unix_socket: /var/lib/headscale/headscale.sock
unix_socket_permission: "0770"
# Clients told not to send their logs to Tailscale.
logtail:
  enabled: false
EOF
    # Every node reaches every other (the trust is wadd's to decide), and
    # sam's machines may share and browse Taildrive between themselves (a
    # real tailnet's default says so with autogroup:member to
    # autogroup:self, which Headscale 0.29.7 takes too). Without the grant a
    # share is 403 and isn't listed; without drive:share, sharing is refused.
    # eve may own tag:device (the device, tagged).
    cat > "${DIR}/etc/policy.hujson" << 'EOF'
{
  "tagOwners": {
    "tag:device": ["eve@"],
  },
  "grants": [
    {"src": ["*"], "dst": ["*"], "ip": ["*"]},
    {
      "src": ["sam@"],
      "dst": ["sam@"],
      "app": {"tailscale.com/cap/drive": [{"shares": ["*"], "access": "rw"}]},
    },
  ],
  "nodeAttrs": [
    {"target": ["sam@"], "attr": ["drive:share", "drive:access"]},
  ],
}
EOF
}

# ------------------------------------------------------------ start, stop

start() {
    command -v openssl > /dev/null || die "no openssl"
    if up; then
        echo "running already (${URL}, on 127.0.0.1:${PORT})"
        return 0
    fi
    stop > /dev/null 2>&1 || true
    podman image exists "${IMAGE}" || podman pull -q "${IMAGE}" > /dev/null
    certs
    config
    podman run --rm -v "${DIR}/etc:/etc/headscale:ro,z" "${IMAGE}" configtest > /dev/null 2>&1 ||
        { podman run --rm -v "${DIR}/etc:/etc/headscale:ro,z" "${IMAGE}" configtest || true; die "Headscale's configtest failed"; }
    # The pod: the laptop's 127.0.0.1 only (the VMs' 10.0.2.2), and hs.test
    # is 127.0.0.1 inside it (for the device).
    # The device's SOCKS5 proxy too, for a browser on the laptop (phase 7).
    podman pod create --name "${POD}" \
        -p "127.0.0.1:${PORT}:${PORT}/tcp" -p "127.0.0.1:${STUN}:${STUN}/udp" \
        -p "127.0.0.1:${SOCKS_PUBLISHED}:${SOCKS##*:}/tcp" \
        --add-host "${HOST}:127.0.0.1" > /dev/null
    podman volume create "${VOLUME}" > /dev/null
    podman run -d --pod "${POD}" --name "${HS}" \
        -v "${DIR}/etc:/etc/headscale:ro,z" -v "${VOLUME}:/var/lib/headscale" \
        "${IMAGE}" serve > /dev/null
    local ok=""
    for _ in $(seq 60); do
        if curl -sf --cacert "${DIR}/ca.crt" --resolve "${HOST}:${PORT}:127.0.0.1" "${URL}/health" > /dev/null 2>&1; then
            ok=1
            break
        fi
        up || break
        sleep 0.5
    done
    if [[ -z "${ok}" ]]; then
        podman logs "${HS}" 2>&1 | tail -n 20 >&2
        die "Headscale didn't come up"
    fi
    user sam > /dev/null
    user eve > /dev/null
    echo "Headscale $(hs version 2> /dev/null | sed -n 's/^headscale version //p') at ${URL} (127.0.0.1:${PORT}), users: $(hs users list -o json | jq -r '[.[]? | "\(.name) (\(.id))"] | join(", ")')"
}

stop() {
    podman rm -f -t 2 "${DEVICE}" "${HS}" > /dev/null 2>&1 || true
    podman pod rm -f "${POD}" > /dev/null 2>&1 || true
    podman volume rm -f "${VOLUME}" > /dev/null 2>&1 || true
    rm -rf "${DIR}"
    echo "stopped (pod, volume and ${DIR} removed)"
}

status() {
    if up; then
        echo "Headscale: running at ${URL} (127.0.0.1:${PORT})"
        hs nodes list -o json 2> /dev/null |
            jq -r '.[]? | "  \(.id) \(.given_name) \(.user.name) \(.ip_addresses[0]) online=\(.online) tags=\(.tags // [] | join(","))"'
    else
        echo "Headscale: not running"
    fi
    podman container exists "${DEVICE}" 2> /dev/null && echo "device: $(podman inspect -f '{{.State.Status}}' "${DEVICE}")"
    return 0
}

# ------------------------------------------------------------ users, keys

user() {
    need_up
    local name="$1" id
    id="$(hs users list -o json | jq -r --arg n "${name}" '.[]? | select(.name == $n) | .id')"
    if [[ -z "${id}" ]]; then
        hs users create "${name}" > /dev/null
        id="$(hs users list -o json | jq -r --arg n "${name}" '.[]? | select(.name == $n) | .id')"
    fi
    [[ -n "${id}" ]] || die "no user ${name}"
    echo "${id}"
}

key() {
    need_up
    [[ $# -ge 1 ]] || die "key USER [--tags tag:x] [--ephemeral]"
    local id
    id="$(user "$1")"
    shift
    hs preauthkeys create --user "${id}" --reusable --expiration 1h "$@" -o json | jq -er .key
}

approve() {
    need_up
    [[ $# -eq 2 ]] || die "approve AUTH-ID USER"
    hs auth register --auth-id "$1" --user "$2"
}

# sam's machines with the "https" node attribute, or without it: the policy
# file changed, and Headscale told to read it again. Without it, `tailscale
# serve --https` asks the control server to turn HTTPS on, and Headscale
# answers 501; with it, the serve is configured, and a TLS client still gets
# no certificate (Headscale can't issue one).
https() {
    need_up
    local p="${DIR}/etc/policy.hujson"
    case "${1:-}" in
        on) grep -q '"drive:access", "https"' "${p}" || sed -i 's/"drive:share", "drive:access"\]/"drive:share", "drive:access", "https"]/' "${p}" ;;
        off) sed -i 's/"drive:share", "drive:access", "https"\]/"drive:share", "drive:access"]/' "${p}" ;;
        *) die "https on|off" ;;
    esac
    podman kill --signal HUP "${HS}" > /dev/null
    echo "https ${1}: $(grep -o '"attr": \[[^]]*\]' "${p}")"
}

# ------------------------------------------------------------ the device

device() {
    local cmd="${1:-}"
    [[ $# -gt 0 ]] && shift
    case "${cmd}" in
        start)
            need_up
            podman rm -f -t 2 "${DEVICE}" > /dev/null 2>&1 || true
            # Its own socket and memory state (an ephemeral node), userspace
            # networking (no TUN, no root): its way onto the tailnet is the
            # SOCKS5 proxy, in the pod (all its addresses: pasta hands the
            # published port to the pod's own, not its loopback; nothing but
            # the pod and that port reaches them). It trusts the test CA only
            # through SSL_CERT_FILE, and sends Tailscale no logs.
            podman run -d --pod "${POD}" --name "${DEVICE}" \
                -v "${DIR}/ca.crt:/etc/wados-test-ca.crt:ro,z" -e SSL_CERT_FILE=/etc/wados-test-ca.crt \
                --entrypoint /usr/sbin/tailscaled "${DEVICE_IMAGE}" \
                --tun=userspace-networking --state=mem: --statedir=/var/lib/ts-device \
                --socket="${DEVICE_SOCK}" --port=0 --no-logs-no-support \
                --socks5-server="0.0.0.0:${SOCKS##*:}" > /dev/null
            for _ in $(seq 40); do
                podman exec "${DEVICE}" tailscale --socket="${DEVICE_SOCK}" status --json > /dev/null 2>&1 && break
                sleep 0.5
            done
            echo "device: $(podman exec "${DEVICE}" tailscale --socket="${DEVICE_SOCK}" version | head -1), $(podman exec "${DEVICE}" tailscale --socket="${DEVICE_SOCK}" status --json | jq -r .BackendState)"
            ;;
        ts) podman exec "${DEVICE}" tailscale --socket="${DEVICE_SOCK}" "$@" ;;
        curl) podman exec "${DEVICE}" curl --proxy "socks5h://${SOCKS}" "$@" ;;
        stop) podman rm -f -t 2 "${DEVICE}" > /dev/null 2>&1 || true ;;
        *) die "device start|ts ARGS…|curl ARGS…|stop" ;;
    esac
}

cmd="${1:-}"
[[ $# -gt 0 ]] && shift
case "${cmd}" in
    start) start ;;
    stop) stop ;;
    status) status ;;
    hs) need_up; hs "$@" ;;
    user) [[ $# -eq 1 ]] || die "user NAME"; user "$1" ;;
    key) key "$@" ;;
    approve) approve "$@" ;;
    https) https "$@" ;;
    socks) echo "127.0.0.1:${SOCKS_PUBLISHED}" ;;
    ca) [[ -f "${DIR}/ca.crt" ]] || die "no CA: tools/tailnet.sh start"; echo "${DIR}/ca.crt" ;;
    url) echo "${URL}" ;;
    device) device "$@" ;;
    *) sed -n '2,36p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
