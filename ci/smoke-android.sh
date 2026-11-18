#!/usr/bin/env bash
# On a running Android TV emulator: install the APK, give it silent test media,
# flip channels with remote keys, take screenshots and keep the log.
set -euo pipefail
apk=$1
out=$2
mkdir -p "$out"
media=$(mktemp -d)
"$(dirname "$0")/make-test-media.sh" "$media"
adb install -r "$apk"
adb shell mkdir -p "/sdcard/Movies/Test\ Pattern" "/sdcard/Movies/Color\ Bars"
adb push "$media/Test Pattern/Test Pattern.mp4" "/sdcard/Movies/Test Pattern/"
adb push "$media/Color Bars/Color Bars.mp4" "/sdcard/Movies/Color Bars/"
adb shell pm grant org.retrorabbit.tv android.permission.READ_MEDIA_VIDEO || true
adb logcat -c
adb shell am start -n org.retrorabbit.tv/.MainActivity
shot() { sleep "$2"; adb exec-out screencap -p > "$out/$1.png"; }
key() { adb shell input keyevent "$1"; }
shot 1-start 30
key KEYCODE_CHANNEL_UP;   shot 2-channel-up 10
key KEYCODE_DPAD_UP;      shot 3-dpad-up 10
key KEYCODE_CHANNEL_DOWN; shot 4-channel-down 10
key KEYCODE_5;            shot 5-typed-5 10
key KEYCODE_BACK;         shot 6-back-recall 10
key KEYCODE_DPAD_CENTER;  shot 7-ok-toolbar 5
key KEYCODE_BACK
key KEYCODE_GUIDE;        shot 8-guide 5
key KEYCODE_BACK
key KEYCODE_PROG_BLUE;    shot 9-channel-editor 5
adb logcat -d > "$out/logcat.txt"
grep -E "RetroRabbit|AndroidRuntime|FATAL|libc" "$out/logcat.txt" | tail -200 > "$out/log-summary.txt" || true
