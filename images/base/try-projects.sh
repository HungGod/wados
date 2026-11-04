#!/usr/bin/env bash
# wadspaces-projects (the image's projects: --init, --sync, --mounts) against
# folders in a temp dir, as you, with no container: a projects.json and
# wadd's copies (files/ and done, as apps/wadd/src/copies.rs stages them)
# made up here. Checks names with spaces, quotes, $, %, a backtick and
# Unicode; a copy of one file arriving as that file; names that stay out;
# each launcher's Exec, run by gio (what the desktop launches with) as it
# would be. Prints "ok" or what went wrong.
#
#   images/base/try-projects.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="${HERE}/root/usr/local/bin/wadspaces-projects"
# As the image runs it (its files are made executable there).
projects() { python3 "${SCRIPT}" "$@"; }
T="$(mktemp -d)"
trap 'rm -rf "${T}"' EXIT
fail() { echo "FAILED: $*"; exit 1; }

DESK="${T}/home/Desktop"
STATE="${T}/home/.wadspaces"
mkdir -p "${DESK}" "${STATE}" "${T}/extra" "${T}/copies" "${T}/apps"
export WADSPACES_PROJECTS_MANIFEST="${T}/extra/projects.json" WADSPACES_COPIES="${T}/copies"
export WADSPACES_LAYOUT="${T}/layout.json" WADSPACES_APPS="${T}/apps" WADSPACES_OPEN="${T}/open"
# Something opens folders in a window (else projects get no launchers).
printf '[Desktop Entry]\nType=Application\nName=Files\nExec=true\nMimeType=inode/directory;\n' > "${T}/apps/files.desktop"
# The launchers' opener: says what it was given.
cat > "${T}/open" << 'EOF'
#!/bin/sh
{ printf '%s\n' "$#"; printf '%s' "$1"; } > "${REC:?}"
EOF
chmod +x "${T}/open"

TRICKY=$'Ünï "dq" \'sq\' $HOME 100% back`tick'
# wadd's copy: a folder's contents, or one file (its done names it).
copy() { # id file-in-files content [one]
    mkdir -p "${T}/copies/$1/files/$(dirname "$2")"
    printf '%s' "$3" > "${T}/copies/$1/files/$2"
    if [[ "${4:-}" == one ]]; then
        python3 -c 'import json,sys; print(json.dumps({"files": 1, "bytes": 4, "from": "m:x", "file": sys.argv[1]}))' "$2" \
            > "${T}/copies/$1/done"
    else
        echo '{"files": 1, "bytes": 4, "from": "m:x"}' > "${T}/copies/$1/done"
    fi
}
copy p-notes a.txt notes
copy p-deck "Pitch deck.pdf" "%PDF" one
copy p-tricky inner.txt tricky
copy p-old "old.txt" "late" one
copy "p-4.x" b.txt dotted
# A mount: wadd's bind mount is there already.
mkdir -p "${DESK}/Thesis Folder"
# An older --init made an empty folder where a file's copy goes.
mkdir -p "${DESK}/old.txt"

TRICKY="${TRICKY}" python3 - "${WADSPACES_PROJECTS_MANIFEST}" << 'EOF'
import json, os, sys
p = lambda id, kind, name: {"id": id, "kind": kind, "mount": name, "name": name}
projects = [
    p("p-notes", "copy", "My Notes"),
    p("p-deck", "copy", "Pitch deck.pdf"),
    p("p-tricky", "copy", os.environ["TRICKY"]),
    p("p-old", "copy", "old.txt"),
    p("p-4.x", "copy", "dotted id"),
    p("p-thesis", "mount", "Thesis Folder"),
    # Kept out: hidden, a path, up a folder, a control character, too long, a bad id.
    p("p-hidden", "copy", ".hidden"),
    p("p-path", "copy", "a/b"),
    p("p-up", "copy", ".."),
    p("p-tab", "copy", "a\tb"),
    p("p-long", "copy", "é" * 128),
    p(".p-bad", "copy", "Bad id"),
]
json.dump({"version": 2, "projects": projects}, open(sys.argv[1], "w"))
EOF

projects --init "${DESK}" "${STATE}" 2> "${T}/init.log" || fail "--init: $(cat "${T}/init.log")"
projects --sync "${DESK}" "${STATE}" 2> "${T}/sync.log" || fail "--sync: $(cat "${T}/sync.log")"

# Copied in, under their names.
[[ "$(cat "${DESK}/My Notes/a.txt")" == notes ]] || fail "My Notes wasn't copied in"
[[ "$(cat "${DESK}/${TRICKY}/inner.txt")" == tricky ]] || fail "the tricky name wasn't copied in"
[[ "$(cat "${DESK}/dotted id/b.txt")" == dotted ]] || fail "an id with a dot was refused"
# One file: that file, not a folder holding it (even over an older empty folder).
[[ -f "${DESK}/Pitch deck.pdf" && "$(cat "${DESK}/Pitch deck.pdf")" == "%PDF" ]] || fail "Pitch deck.pdf isn't the file"
[[ -f "${DESK}/old.txt" && "$(cat "${DESK}/old.txt")" == late ]] || fail "old.txt isn't the file"
# Nothing of the names kept out: the Desktop has the projects and their launchers, no more.
got="$(find "${DESK}" -mindepth 1 -maxdepth 1 ! -name '*.desktop' -printf '%f\n' | LC_ALL=C sort)"
want="$(printf '%s\n' "My Notes" "Pitch deck.pdf" "${TRICKY}" old.txt "dotted id" "Thesis Folder" | LC_ALL=C sort)"
[[ "${got}" == "${want}" ]] || fail "the Desktop has: ${got}"
for id in p-hidden p-path p-up p-tab p-long .p-bad; do
    grep -qF "${id}" "${T}/init.log" || fail "${id} wasn't said to be skipped"
    compgen -G "${DESK}/*-project-${id}.desktop" > /dev/null && fail "${id} got a launcher"
done
# --mounts: the projects' names, one a line.
mapfile -t names < <(projects --mounts 2> /dev/null)
[[ "${#names[@]}" -eq 6 && "${names[2]}" == "${TRICKY}" ]] || fail "--mounts said: ${names[*]}"

# Each launcher: its Name, and its Exec as gio runs it (the path, one argument).
n=0
for id in p-notes p-deck p-tricky p-old p-4.x p-thesis; do
    f="$(compgen -G "${DESK}/*-project-${id}.desktop")" || fail "${id} has no launcher"
    name="$(python3 -c 'import json,sys; print(next(p["mount"] for p in json.load(open(sys.argv[1]))["projects"] if p["id"] == sys.argv[2]))' \
        "${WADSPACES_PROJECTS_MANIFEST}" "${id}")"
    grep -qxF "Name=${name}" "${f}" || fail "${id}'s Name: $(grep ^Name= "${f}")"
    if command -v gio > /dev/null; then
        rm -f "${T}/rec"
        REC="${T}/rec" gio launch "${f}" 2> /dev/null || fail "gio couldn't launch ${id}: $(grep ^Exec= "${f}")"
        for _ in $(seq 50); do [[ -s "${T}/rec" ]] && break; sleep 0.1; done
        want="$(printf '1\n%s' "${DESK}/${name}")"
        [[ "$(cat "${T}/rec" 2> /dev/null)" == "${want}" ]] || fail "${id}'s Exec ran with: $(cat "${T}/rec" 2> /dev/null) ($(grep ^Exec= "${f}"))"
        n=$((n + 1))
    fi
done

# Again: nothing copied over what's there.
printf 'changed' > "${T}/copies/p-deck/files/Pitch deck.pdf"
rm -f "${STATE}/copied/p-deck"
printf 'mine' > "${DESK}/Pitch deck.pdf"
projects --sync "${DESK}" "${STATE}" 2> /dev/null
[[ "$(cat "${DESK}/Pitch deck.pdf")" == mine ]] || fail "a file there was copied over"

echo "ok ($n launchers run by gio)"
