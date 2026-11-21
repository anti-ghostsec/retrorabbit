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
# Screen size as the app sees it (landscape), for taps and swipes.
sleep 5
size=$(adb shell dumpsys window | grep -o 'cur=[0-9]*x[0-9]*' | head -1 | cut -d= -f2)
[ -n "$size" ] || size=$(adb shell wm size | grep -o '[0-9]*x[0-9]*' | tail -1)
echo "screen: $size"
w=${size%x*}; h=${size#*x}; cx=$((w / 2)); cy=$((h / 2))
shot 0-menu-button 3
shot 1-start 22
key KEYCODE_CHANNEL_UP;   shot 2-channel-up 10
key KEYCODE_DPAD_UP;      shot 3-dpad-up 10
key KEYCODE_CHANNEL_DOWN; shot 4-channel-down 10
key KEYCODE_5;            shot 5-typed-5 10
key KEYCODE_BACK;         shot 6-back-recall 10
key KEYCODE_DPAD_CENTER;  shot 7-ok-toolbar 5
key KEYCODE_BACK
key KEYCODE_MENU;         shot 8-guide 5
key KEYCODE_BACK
key KEYCODE_PROG_BLUE;    shot 9-channel-editor 5
key KEYCODE_DPAD_DOWN;    shot 10-editor-dpad-focus 3
key KEYCODE_DPAD_DOWN;    shot 10b-editor-dpad-down 3
key KEYCODE_BACK
# Emulator system dialogs ("isn't responding", full-screen hints) would eat the taps.
adb shell am broadcast -a android.intent.action.CLOSE_SYSTEM_DIALOGS >/dev/null || true
sleep 1
# Touch (phones): tap = toolbar, tap again = hide, long-press = guide, swipe = channel.
adb shell input tap "$cx" "$cy";                               shot 11-tap-toolbar 4
adb shell input tap "$cx" "$cy";                               shot 12-tap-hide 4
adb shell input swipe "$cx" "$cy" "$cx" "$cy" 1200;            shot 13-long-press-guide 4
key KEYCODE_BACK
adb shell input swipe "$cx" $((h * 3 / 4)) "$cx" $((h / 4)) 300; shot 14-swipe-up 10
adb logcat -d > "$out/logcat.txt"
grep "focus:" "$out/logcat.txt" > "$out/focus.txt" || true
grep -E "RetroRabbit|AndroidRuntime|FATAL|libc" "$out/logcat.txt" | tail -200 > "$out/log-summary.txt" || true
