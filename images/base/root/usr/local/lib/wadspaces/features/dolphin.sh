# Dolphin: Fedora's dolphin-emu. `wadspaces-dolphin` starts it, giving a new
# home the pad mapping for Play together (wadspaces-gcpad) the first time.
dnf_install dolphin-emu
cat > /usr/local/bin/wadspaces-dolphin <<'WRAP'
#!/bin/sh
dir="${XDG_CONFIG_HOME:-${HOME}/.config}/dolphin-emu"
if [ ! -e "${dir}/GCPadNew.ini" ]; then
    mkdir -p "${dir}"
    /usr/local/lib/wadspaces/wadspaces-gcpad > "${dir}/GCPadNew.ini"
fi
exec dolphin-emu "$@"
WRAP
chmod 755 /usr/local/bin/wadspaces-dolphin
sed -i 's/^Exec=dolphin-emu/Exec=wadspaces-dolphin/' /usr/share/applications/dolphin-emu.desktop
dolphin-tool header --help > /dev/null 2>&1 || true
rpm -q dolphin-emu
# Play together's mapping: port N is Selkies' pad N, as one Xbox-to-GameCube
# layout (A/B/X/Y, RB to Z, the triggers to analog L and R, the right stick
# to the C-stick). The device names are Selkies' pad name, numbered as Dolphin
# numbers devices of one name. Not seen working with a game yet.
cat > /usr/local/lib/wadspaces/wadspaces-gcpad <<'GCPAD'
#!/bin/sh
for n in 1 2 3 4; do
    cat <<INI
[GCPad${n}]
Device = evdev/$((n - 1))/Microsoft X-Box 360 pad
Buttons/A = SOUTH
Buttons/B = EAST
Buttons/X = NORTH
Buttons/Y = WEST
Buttons/Z = TR
Buttons/Start = START
Main Stick/Up = \`Axis 1-\`
Main Stick/Down = \`Axis 1+\`
Main Stick/Left = \`Axis 0-\`
Main Stick/Right = \`Axis 0+\`
C-Stick/Up = \`Axis 4-\`
C-Stick/Down = \`Axis 4+\`
C-Stick/Left = \`Axis 3-\`
C-Stick/Right = \`Axis 3+\`
Triggers/L-Analog = \`Axis 2+\`
Triggers/R-Analog = \`Axis 5+\`
D-Pad/Up = \`Axis 7-\`
D-Pad/Down = \`Axis 7+\`
D-Pad/Left = \`Axis 6-\`
D-Pad/Right = \`Axis 6+\`
INI
done
GCPAD
chmod 755 /usr/local/lib/wadspaces/wadspaces-gcpad
