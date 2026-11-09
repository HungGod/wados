# Ori: OpenRouter's command-line coding agent (ori), a standalone binary
# from OpenRouterLabs/ori-releases. It runs Claude Code, Codex and the rest
# through OpenRouter; it signs in with `ori login`, which keeps the key in
# its own profile, not in the image. The release's SHA256SUMS verifies the
# download, as its install.sh does, and the binary comes gzipped.
case "$(uname -m)" in
    x86_64) ori_asset=ori-linux-x64 ;;
    aarch64) ori_asset=ori-linux-arm64 ;;
    *) echo "ori: no build for $(uname -m)" >&2; exit 1 ;;
esac
ori_base="https://github.com/OpenRouterLabs/ori-releases/releases/latest/download"
ori_tmp="$(mktemp -d)"
curl -fsSL "${ori_base}/SHA256SUMS" -o "${ori_tmp}/SHA256SUMS"
curl -fsSL "${ori_base}/${ori_asset}.gz" -o "${ori_tmp}/${ori_asset}.gz"
(cd "${ori_tmp}" && sha256sum --check --ignore-missing SHA256SUMS)
gzip -dc "${ori_tmp}/${ori_asset}.gz" > "${ori_tmp}/${ori_asset}"
install -m 0755 "${ori_tmp}/${ori_asset}" /usr/local/bin/ori
rm -rf "${ori_tmp}"
unset ori_asset ori_base ori_tmp
ORI_NO_UPDATE_CHECK=1 ori --version > /dev/null
