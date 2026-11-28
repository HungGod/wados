## Memory policy

Do not create, update, or store persistent memories, memory files, or session notes in this project unless the user tells you to. Use the current conversation and inspect the codebase for context; treat the code as the source of truth.

## Updating the machine from this wadspace

You work in the WadOS wadspace (id `wadspaces`): a container on the machine with this repo cloned at `/config/Desktop/WadSpaces`, its own rootless podman (as user `abc`, `HOME=/config`), and the `wadspaces` MCP server on `/run/wadspaces-agents/mcp.sock`. The machine runs WadOS as a bootc image. You update it by building a new host image here and letting the machine switch to it. You never change the machine directly: wadd and the person do that.

1. **Check the change.** Run what CI runs, and fix anything that fails:
   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
2. **Build the host image**, in this wadspace's own podman (no sudo):
   ```bash
   host/build.sh image          # → localhost/wados-host:latest
   ```
   It builds the binaries in `localhost/wados-host-builder:44`, then the image from `host/Containerfile`, and labels it with version `44.<UTC date>.<HHMM>` (or `WADOS_VERSION`). `RECHUNK=0` skips rechunking, which is quicker for a try. Check it's there with `podman images localhost/wados-host`.
3. **Hand it to the person.** wadd finds `localhost/wados-host*` images in running wadspaces' podman by itself, but only offers one that's newer than what's booted. Tell the person with the MCP `notify` tool, e.g. "WadOS 44.20261123.1405 is built: Settings ▸ Updates ▸ This machine". They pick **This machine** as the source, then **Download**. wadd copies the image out (`podman save` here, `podman load` on the machine), then stages it with `bootc switch`. It applies at the next restart, and restarting is always the person's choice. The previous version stays as the rollback.

**Pushing to GitHub instead** (the **Pushed to GitHub** source, for the person's other machines): `podman login ghcr.io`, then `host/build.sh push` (`:latest` and a `<date>-<commit>` tag on `ghcr.io/hunggod/wados-host`). That publishes, so do it only when the person has said to ship, the same as `git push` and `gh pr`. Never push a `--test-key` image (the script refuses).

What's not a path: a sandbox (`request_sandbox`/`update_sandbox`) is a throwaway wadspace for trying an app you built. Files are copied into it, never out of it, and it can't update the machine or this wadspace.
