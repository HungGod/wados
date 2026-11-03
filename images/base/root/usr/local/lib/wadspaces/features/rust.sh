# Rust: rustup (Fedora's rustup-init) and the stable toolchain in
# /usr/local/rustup and /usr/local/cargo. Login shells get them on PATH from
# /etc/profile.d; the desktop's terminals and editors through small wrappers
# in /usr/local/bin (they aren't login shells). `cargo install` goes to
# ~/.cargo, the user's.
need cpp
dnf_install rustup
export RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
rustup-init -y --no-modify-path --profile default --default-toolchain stable
chmod -R a+rX /usr/local/rustup /usr/local/cargo
for bin in /usr/local/cargo/bin/*; do
    name="$(basename "${bin}")"
    cat > "/usr/local/bin/${name}" <<WRAP
#!/bin/sh
export RUSTUP_HOME="\${RUSTUP_HOME:-/usr/local/rustup}"
exec /usr/local/cargo/bin/${name} "\$@"
WRAP
    chmod 755 "/usr/local/bin/${name}"
done
cat > /etc/profile.d/wadspaces-rust.sh <<'PROFILE'
export RUSTUP_HOME=/usr/local/rustup
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"
PROFILE
/usr/local/bin/rustc --version
