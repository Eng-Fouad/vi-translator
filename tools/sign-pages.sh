#!/usr/bin/env bash
# Sign a game-pages script for delivery in a layout update, and print the "pages" block to put into
# that game's layout JSON (see src-tauri/src/pagescript.rs).
#
#   tools/sign-pages.sh [--test [install-id ...] --] <script.js> [path/to/update-signing.pem] > pages-block.json
#
# The app runs a downloaded script only if this signature matches its built-in key (the same key that
# signs app updates), only if the script's `// pages-version: N` is newer than the one it carries, and
# only if it needs no newer app (`// requires-app: X.Y.Z` in its first lines).
#
# "pages" is a list, newest first: the app takes the first entry it can use. To keep older apps
# working when a script needs a newer app, put the previous entry after the new one. --test marks
# the entry for testers only: installs with "Get test versions of game pages" on, and the install
# ids given.
set -euo pipefail
channel=""; installs=()
if [ "${1:-}" = "--test" ]; then
  channel="test"; shift
  while [ $# -gt 0 ] && [ "$1" != "--" ]; do installs+=("$1"); shift; done
  [ "${1:-}" = "--" ] && shift
fi
script="${1:?usage: sign-pages.sh [--test [install-id ...] --] <script.js> [key.pem]}"
key="${2:-$HOME/.vi-translator-keys/update-signing.pem}"
[ -f "$key" ] || { echo "no signing key at $key" >&2; exit 1; }
head -1 "$script" | grep -q '^// pages-version: [0-9]\+' || { echo "the script's first line must be // pages-version: N" >&2; exit 1; }
# (on Windows "python3" may be only a Microsoft Store shortcut: take the first one that runs)
py=""
for c in python3 python; do if command -v "$c" >/dev/null && "$c" -c "" 2>/dev/null; then py=$c; break; fi; done
[ -n "$py" ] || { echo "needs python" >&2; exit 1; }
sig=$(openssl pkeyutl -sign -rawin -inkey "$key" -in "$script" | base64 | tr -d '\r\n')
"$py" -c '
import json, sys
e = {"script": open(sys.argv[1], encoding="utf8", newline="").read(), "sig": sys.argv[2]}
if sys.argv[3]:
    e["channel"] = sys.argv[3]
    if sys.argv[4:]:
        e["installs"] = sys.argv[4:]
print(json.dumps({"pages": [e]}, ensure_ascii=False, indent=2))
' "$script" "$sig" "$channel" "${installs[@]}"
