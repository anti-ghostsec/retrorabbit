#!/usr/bin/env bash
# Two short, silent test videos in two folders (= two local channels).
# Usage: make-test-media.sh DIR [ffmpeg]
set -euo pipefail
dir=$1
ff=${2:-ffmpeg}
# H.264 where the ffmpeg build has it; LGPL builds (Windows) only have MPEG-4.
if "$ff" -nostdin -hide_banner -encoders 2>/dev/null | grep libx264 >/dev/null; then
  codec=(-c:v libx264 -preset veryfast -pix_fmt yuv420p)
else
  codec=(-c:v mpeg4 -q:v 4 -pix_fmt yuv420p)
fi
mkdir -p "$dir/Test Pattern" "$dir/Color Bars"
"$ff" -nostdin -loglevel error -y -f lavfi -i testsrc2=size=640x360:rate=25 -t 300 -an \
  "${codec[@]}" "$dir/Test Pattern/Test Pattern.mp4"
"$ff" -nostdin -loglevel error -y -f lavfi -i smptebars=size=640x360:rate=25 -t 300 -an \
  "${codec[@]}" "$dir/Color Bars/Color Bars.mp4"
