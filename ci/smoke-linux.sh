#!/usr/bin/env bash
# Run RetroRabbit under Xvfb on silent test media, flip channels, take screenshots.
set -euo pipefail
bin=$1
out=$2
mkdir -p "$out"
media=$(mktemp -d)
"$(dirname "$0")/make-test-media.sh" "$media"
export DISPLAY=:99
Xvfb :99 -screen 0 1280x800x24 &
sleep 2
"$bin" --channel 2 "$media" >"$out/app.log" 2>&1 &
app=$!
shot() { sleep "$2"; import -window root "$out/$1.png"; }
shot 1-start 20
kill -0 "$app" || { echo "RetroRabbit exited:"; cat "$out/app.log"; exit 1; }
win=$(xdotool search --name '^RetroRabbit$' | head -1 || true)
[ -n "$win" ] || { echo "no RetroRabbit window"; cat "$out/app.log"; exit 1; }
xdotool windowactivate --sync "$win" || true
xdotool key --window "$win" Page_Up;   shot 2-channel-up 8
xdotool key --window "$win" Page_Down; shot 3-channel-down 8
xdotool type --window "$win" 3;        shot 4-typed-3 8
xdotool key --window "$win" g;         shot 5-guide 5
kill "$app"
wait "$app" || true
