# tools/vm.sh agents-check: the spike's agents demo in a real wadspace, on the
# real MCP socket, with SELinux on (sourced after tools/vm/check.sh). What's
# fake is `claude` (tools/vm/fake-claude): no model and no account.

ag() { api GET /v1/agents; }
agent_is() { [[ "$(ag | jq -r '.agents[0].status')" == "$1" ]]; }
sandbox_state() { ag | jq -r '.sandboxes[0].state'; }
sandbox_at() { [[ "$(ag | jq -r '.sandboxes[0].build')" == "$1" && "$(sandbox_state)" == ready ]]; }
inner() { as_root "podman exec --user abc wad-$1 sh -c '$2'"; }
expect_is() { if [[ "$3" == "$2" ]]; then ok "$1"; else fail "$1: wanted “$2”, got “${3:0:220}”"; fi; }

check_demo() {
    say "the demo"
    local id
    id="$(api POST /v1/ops '{"op": {"op": "space.create", "name": "Work"}}' | jq -r .created)"
    api POST /v1/ops "{\"op\": {\"op\": \"apps.add\", \"ws\": \"${id}\", \"app\": \"terminal\"}}" > /dev/null
    api POST /v1/ops "{\"op\": {\"op\": \"builder.agents\", \"ws\": \"${id}\", \"on\": true}}" > /dev/null
    api POST "/v1/wadspaces/${id}/build" > /dev/null
    api POST "/v1/wadspaces/${id}/open" '{"display": "background"}' > /dev/null
    wait_for 120 ready "${id}" && ok "the wadspace is up" || fail "it didn't start"
    # After it's built: `claude-code` is a package, and packages aren't published yet.
    api POST /v1/ops "{\"op\": {\"op\": \"builder.add\", \"ws\": \"${id}\", \"section\": \"addons\", \"item\": \"terminal:claude-code\"}}" > /dev/null
    as_root "cat > /tmp/claude; chmod 755 /tmp/claude; podman cp /tmp/claude wad-${id}:/usr/local/bin/claude" < "${HERE}/fake-claude"
    expect_is "its socket is there" yes "$(inner "${id}" 'test -S /run/wadspaces-agents/mcp.sock && echo yes')"
    local reply
    reply="$(api POST /v1/agents "{\"ws\": \"${id}\", \"text\": \"Pin wadspaces to the panel\"}")"
    expect_is "the job started" "Pin wadspaces to the panel" "$(jq -r .task <<< "${reply}")"
    wait_for 40 agent_is needs-you && ok "it asks for a sandbox" || { fail "it stopped: $(ag | jq -c '.agents[0] | {status, now}')"; return; }
    local a; a="$(ag | jq -r '.agents[0].id')"
    expect_is "what it asked" sandbox "$(ag | jq -r '.agents[0].ask.kind')"
    expect_is "its work shows" "Edit src/pins.rs" "$(ag | jq -r '.agents[0].notes[] | select(.kind == "edit") | "Edit \(.path)"')"
    expect_is "nothing's made until it's allowed" null "$(ag | jq -r '.sandboxes[0]')"
    expect_is "a turn can't push yet" 1 "$(inner "${id}" 'grep -c disallowedTools /tmp/claude-args')"
    expect_is "the tray counts it" "1" "$(ag | jq '[.agents[] | select(.status == "needs-you")] | length')"
    note "allow: $(api POST "/v1/agents/${a}/allow" '{}' | cut -c1-300)"
    b1() { sandbox_at 1; }
    wait_for 120 b1 && ok "the sandbox has build 1" || { fail "no build 1: $(ag | jq -c '.sandboxes[0]')"; return; }
    local box; box="$(ag | jq -r '.sandboxes[0].ws')"
    wait_for 20 test -n "$(inner "${box}" 'cat /tmp/ran.txt 2>/dev/null')"
    expect_is "it ran in the sandbox" build1 "$(inner "${box}" 'cat /tmp/ran.txt')"
    expect_is "the sandbox can't see its keys" 0 "$(api GET /v1/library | jq --arg b "${box}" '[.wadspaces[] | select(.id == $b) | .secrets.bySlot | length] | add // 0')"
    expect_is "it has no clipboard of the machine's" 0 "$(as_root "journalctl -u wadd --no-pager | grep -c '${box}: its clipboard is'")"
    expect_is "nor reach the MCP server" no "$(inner "${box}" 'test -S /run/wadspaces-agents/mcp.sock && echo yes || echo no')"
    local s; s="$(ag | jq -r '.sandboxes[0].id')"
    api POST "/v1/sandboxes/${s}/mark" '{"item": "menu", "verdict": "works"}' > /dev/null
    api POST "/v1/sandboxes/${s}/mark" '{"item": "once", "verdict": "issue", "note": "It pinned twice"}' > /dev/null
    api POST "/v1/sandboxes/${s}/send" '{}' > /dev/null
    b2() { sandbox_at 2; }
    wait_for 60 b2 && ok "build 2 arrives" || fail "no build 2: $(ag | jq -c '.sandboxes[0]')"
    expect_is "what it changed is asked about again" true "$(ag | jq -r '.sandboxes[0].items[] | select(.id == "once") | .fresh')"
    expect_is "what didn't keeps its verdict" works "$(ag | jq -r '.sandboxes[0].items[] | select(.id == "menu") | .verdict')"
    expect_is "it heard the note" 1 "$(inner "${id}" 'grep -c "It pinned twice" /tmp/claude-args')"
    expect_is "it hasn't pushed" no "$(inner "${id}" 'test -e /tmp/pushed && echo yes || echo no')"
    api POST "/v1/agents/${a}/ship" '{}' > /dev/null
    wait_for 40 agent_is done && ok "shipped, when told to" || fail "not done: $(ag | jq -c '.agents[0] | {status, now}')"
    expect_is "only then could it push" 3 "$(inner "${id}" 'grep -c disallowedTools /tmp/claude-args')"
    expect_is "the last turn could" 4 "$(inner "${id}" 'wc -l < /tmp/claude-args' | tr -d ' ')"
    expect_is "the sandbox is gone" closed "$(sandbox_state)"
    gone() { [[ "$(api GET /v1/library | jq --arg b "${box}" '[.wadspaces[] | select(.id == $b)] | length')" == 0 ]]; }
    wait_for 40 gone && ok "its wadspace is gone" || fail "its wadspace stayed"
    api POST /v1/ops "{\"op\": {\"op\": \"builder.agents\", \"ws\": \"${id}\", \"on\": false}}" > /dev/null
    sleep 4
    expect_is "the switch off takes the socket" no "$(inner "${id}" 'test -S /run/wadspaces-agents/mcp.sock && echo yes || echo no')"
    shot_to 20-agents.png
    # The real client, no account: it still says if it reached the server.
    api POST /v1/ops "{\"op\": {\"op\": \"builder.agents\", \"ws\": \"${id}\", \"on\": true}}" > /dev/null
    sleep 4
    as_root "podman exec wad-${id} sh -c 'rm -f /usr/local/bin/claude; wadspaces-feature claude-code'" > /dev/null 2>&1
    expect_is "the real client installs" 1 "$(inner "${id}" 'claude --version | grep -c "Claude Code"')"
    local init
    init="$(as_root "podman exec --user abc -e HOME=/config wad-${id} claude -p hi --output-format stream-json --verbose --mcp-config /run/wadspaces-extra/mcp.json --strict-mcp-config" | head -1)"
    expect_is "and it reaches our MCP server" connected "$(jq -r '.mcp_servers[] | select(.name == "wadspaces") | .status' <<< "${init}")"
    expect_is "and sees its tools" 5 "$(jq '[.tools[] | select(startswith("mcp__wadspaces__"))] | length' <<< "${init}")"
}

ready() { [[ "$(run_phase "$1")" == ready ]]; }

agents_check() {
    set +e
    FAILED=0
    SHOTS="${VM}/agents-shots"
    mkdir -p "${SHOTS}"
    rm -f "${SHOTS:?}"/*.png
    local t0 port
    t0=$(now)
    STEP_T=$(now)
    stop > /dev/null
    boot --fresh
    port="$(cat "${VM}/ssh.port")"
    wait_ssh "${port}" root 420 "${VM}/qemu.pid" || { fail "no SSH"; finish "${t0}"; return; }
    check_user
    unlock && ok "unlocked" || fail "didn't unlock"
    check_demo
    finish "${t0}"
}
