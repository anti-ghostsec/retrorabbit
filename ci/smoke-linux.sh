#!/usr/bin/env bash
# Run RetroRabbit under Xvfb on silent test media, flip channels, take screenshots.
set -euxo pipefail
bin=$1
out=$2
mkdir -p "$out"
media=$(mktemp -d)
"$(dirname "$0")/make-test-media.sh" "$media"
export DISPLAY=:99
Xvfb :99 -screen 0 1280x800x24 >/dev/null 2>&1 &
sleep 2
"$bin" --channel 2 "$media" >"$out/app.log" 2>&1 &
app=$!
shot() { sleep "$2"; timeout 20 ffmpeg -nostdin -loglevel error -y -f x11grab -video_size 1280x800 -i :99 -frames:v 1 "$out/$1.png" || echo "screenshot $1 failed"; }
shot 1-start 20
kill -0 "$app" || { echo "RetroRabbit exited:"; cat "$out/app.log"; exit 1; }
win=$(timeout 10 xdotool search --name '^RetroRabbit$' | head -1 || true)
[ -n "$win" ] || { echo "no RetroRabbit window"; cat "$out/app.log"; exit 1; }
timeout 5 xdotool windowactivate "$win" || true  # no window manager under Xvfb
timeout 10 xdotool key --window "$win" Page_Up;   shot 2-channel-up 8
timeout 10 xdotool key --window "$win" Page_Down; shot 3-channel-down 8
timeout 10 xdotool type --window "$win" 5;        shot 4-typed-5 8
timeout 10 xdotool key --window "$win" g;         shot 5-guide 5
kill "$app" || true
sleep 2
kill -9 "$app" 2>/dev/null || true
pkill -x mpv || true
