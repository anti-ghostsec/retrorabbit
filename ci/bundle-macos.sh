#!/usr/bin/env bash
# Build RetroRabbit.app: our binary, libmpv and ffprobe (from Homebrew) with all
# their libraries copied in and relinked, then ad-hoc signed.
set -euo pipefail
bin=$1
app=$2
version=$3
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/retrorabbit"
prefix=$(brew --prefix)
cp "$prefix/lib/libmpv.2.dylib" "$app/Contents/Frameworks/"
cp "$prefix/bin/ffprobe" "$app/Contents/MacOS/"
chmod u+w "$app/Contents/Frameworks/libmpv.2.dylib" "$app/Contents/MacOS/ffprobe"
install_name_tool -id @rpath/libmpv.2.dylib "$app/Contents/Frameworks/libmpv.2.dylib"
dylibbundler -of -b -cd \
  -x "$app/Contents/Frameworks/libmpv.2.dylib" \
  -x "$app/Contents/MacOS/ffprobe" \
  -d "$app/Contents/Frameworks" -p @executable_path/../Frameworks/ \
  -s "$prefix/lib"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>RetroRabbit</string>
  <key>CFBundleDisplayName</key><string>RetroRabbit</string>
  <key>CFBundleIdentifier</key><string>org.retrorabbit.desktop</string>
  <key>CFBundleExecutable</key><string>retrorabbit</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>$(sw_vers -productVersion | cut -d. -f1).0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.entertainment</string>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$app"
