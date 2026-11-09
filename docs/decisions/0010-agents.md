# 0010: agents and sandboxes

Phase 11. Claude Code works in a wadspace, messages the desktop over the `wadspaces` MCP server, asks for a sandbox, and
ships only when the person says so.

## A job

- **Real Claude Code, in its wadspace:** `podman exec` of `claude -p` with `--output-format stream-json --verbose`,
  turn by turn, with `--resume`. `--bare` isn't used: it ignores a subscription's login.
- **Between turns it waits for the person:** a sandbox to allow, one to try, or a pull request.
- **Nothing ships by itself:** until they say so, `git push` and `gh pr` are `--disallowedTools`. It's a guard on Claude
  Code's tools, not a security boundary: the wadspace has its own credentials, and a script could still push.
- **Permissions:** `acceptEdits`, with Bash and the server's tools allowed. A headless run denies whatever would prompt.
- **Stopping** is SIGINT, which ends the turn; SIGTERM would leave it unfinished.
- **Failures come as a result, and exit 1.** A failed login reports `"subtype":"success"` with `"is_error":true`, so both
  are read. Seen on the real client.

## The `wadspaces` MCP server

- **A socket per wadspace with its Agents switch on,** so the caller is known by the socket, never by what it says. A
  wadspace with it off has none. `wadspaces-mcp` in the base relays Claude Code's stdio to it.
- **Tools:** `notify`, `progress`, `request_sandbox`, `set_checklist`, `update_sandbox`.
- **What comes over it is untrusted:** cut to size, and shown as data. Approvals happen on the host, never in the
  container.
- **`request_sandbox` answers at once,** and the agent ends its turn: the person's answer resumes it. So no call hangs for
  hours waiting for someone.
- **The socket is labelled as a screen's** (`wad_wayland`'s `bind`), or SELinux stops a container even stat'ing it.

## On every machine

- **Notices go round.** Every notice a machine tells is posted to each of the owner's other online machines
  (`POST /v1/remote/notices`, my machines only), and shown there as that machine's ("Studio: Claude Code wants a
  sandbox"), once: a notice's id is a hash of what it says, and one that came from another machine is never passed on
  again.
- **An allowed sandbox streams to whoever accepted it.** The accept (`POST /v1/agents/{id}/allow`) says where it's
  shown: `local` is a window on this machine (the person is here); `stream` builds and runs it **here** (the agent's
  own machine, whose files it tries), off the screen, and answers a private tailnet stream's one-tap link for the
  accepter (a phone's `StreamActivity`, another machine's WadBrowser). Nothing is elected or moved: the accepter is
  whoever asked.

## Sandboxes

- **A throwaway wadspace, not a host user, which the spec had.** Code an agent just wrote never runs on the machine.
- **It's `sealed`:** no keys, no projects, no MCP server, and no clipboard bridge. Every other wadspace on the screen shares
  the machine's clipboard, so that needed saying.
- **`update_sandbox`** pipes `tar` between the two wadspaces, as each one's own user, because `podman cp` can't write into
  an id-mapped `/config`. Then it runs its command on the sandbox's desktop.
- **The checklist** is in the Agents app. A new build keeps verdicts on what didn't change, and asks again about what did.

## Running podman from wadd

`wadd.service` has `ProtectSystem=strict`, so the podman command line fails inside it (exit 125). `updates::outside` runs
a unit of systemd's instead, its output root's alone. Dolphin's and Steam's pushes had this bug too: fake registries hid
it, and they use this now.

## The UI

`apps/wados-agents`: the overview, the rail, a job in full, and the checklist. The tray's icon has a count and a flyout,
and Review on a toast opens the job. The Agents switch is in a wadspace's menu.

## Testing

- **`tools/vm.sh agents-check`,** the demo on a real machine with SELinux on. `tools/vm/fake-claude` prints the documented
  stream and calls the real socket through the real relay: request, allow, build 1, feedback, build 2, ship, teardown.
- **The real client, 2.1.318,** with no account: it installs, its flags exist, and its `system/init` says it reached the
  server and sees all five tools.
- **Unit tests** cover the stream, the protocol and the rules, including the real not-logged-in line.

## Not yet

- **No real job has run.** It needs the person's sign-in and spends their usage, so no model has ever driven this. Whether
  Claude uses the tools as `HOW` asks is unknown.
- **The spike's parts not built:** attachments, a word to a job while it's working, the ledger, step-by-step editor following
  in VS Code, the ring around the tray icon, `+N −N` on edits, and the chat's terminal look.
- **Pull requests aren't tracked:** there's no `Pull request #N` card, only what the agent says.
- **The sandbox isn't a second WadSpaces.** It's the wadspace's own desktop with the build running in it. Testing the shell
  itself needs that build to start a nested session.
- **A wadspace's agents can read its own files and use its own sign-ins.** That's the point of them, and why the guard on
  pushing isn't a boundary.
