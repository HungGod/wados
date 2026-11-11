# Claude Code's command line, Anthropic's native build (claude.ai/install.sh,
# no Node.js): what the Agents app runs in a wadspace, and a tool of its own
# in a terminal. The installer sets it up in a home of its own; only the
# binary is kept, at /usr/local/bin/claude. It's updated as a package (CI
# builds it again), so it doesn't update itself: managed settings say so.
# It signs in with its key; nothing about an account is in the image.
claude_home="$(mktemp -d)"
curl -fsSL https://claude.ai/install.sh -o "${claude_home}/install.sh"
HOME="${claude_home}" bash "${claude_home}/install.sh"
install -m 0755 "$(readlink -f "${claude_home}/.local/bin/claude")" /usr/local/bin/claude
rm -rf "${claude_home}"
unset claude_home
install -d -m 0755 /etc/claude-code
cat > /etc/claude-code/managed-settings.json << 'JSON'
{"env": {"DISABLE_AUTOUPDATER": "1"}}
JSON
DISABLE_AUTOUPDATER=1 claude --version
