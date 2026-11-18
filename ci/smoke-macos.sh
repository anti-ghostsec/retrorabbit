#!/usr/bin/env bash
# Run RetroRabbit.app on silent test media, flip channels, take screenshots.
set -euo pipefail
app=$1
out=$2
mkdir -p "$out"
media=$(mktemp -d)
"$(dirname "$0")/make-test-media.sh" "$media"
"$app/Contents/MacOS/retrorabbit" --channel 2 "$media" &
pid=$!
shot() { sleep "$2"; screencapture -x "$out/$1.png" || true; }
key() { osascript -e "tell application \"System Events\" to $1" || echo "couldn't send key: $1"; }
shot 1-start 25
osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $pid) to true" || true
key "key code 116"; shot 2-channel-up 8      # Page Up
key "key code 121"; shot 3-channel-down 8    # Page Down
key 'keystroke "3"'; shot 4-typed-3 8
key 'keystroke "g"'; shot 5-guide 5
kill "$pid" || true
