#!/usr/bin/env bash
# Two short, silent test videos in two folders (= two local channels).
# Usage: make-test-media.sh DIR [ffmpeg]
set -euo pipefail
dir=$1
ff=${2:-ffmpeg}
mkdir -p "$dir/Test Pattern" "$dir/Color Bars"
"$ff" -loglevel error -y -f lavfi -i testsrc2=size=640x360:rate=25 -t 300 -an \
  -c:v libx264 -preset veryfast -pix_fmt yuv420p "$dir/Test Pattern/Test Pattern.mp4"
"$ff" -loglevel error -y -f lavfi -i smptebars=size=640x360:rate=25 -t 300 -an \
  -c:v libx264 -preset veryfast -pix_fmt yuv420p "$dir/Color Bars/Color Bars.mp4"
