#!/usr/bin/env bash
# Sign a game-pages script for delivery in a layout update, and print the "pages" block to put into
# that game's layout JSON (see src-tauri/src/pagescript.rs).
#
#   tools/sign-pages.sh src-tauri/pages/gta5.js [path/to/update-signing.pem] > pages-block.json
#
# The app runs a downloaded script only if this signature matches its built-in key (the same key that
# signs app updates), and only if the script's `// pages-version: N` is newer than the one it carries.
set -euo pipefail
script="${1:?usage: sign-pages.sh <script.js> [key.pem]}"
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
print(json.dumps({"pages": {"script": open(sys.argv[1], encoding="utf8", newline="").read(), "sig": sys.argv[2]}}, ensure_ascii=False, indent=2))
' "$script" "$sig"
