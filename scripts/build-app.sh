#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
mode="${GITFLOW_BUILD_MODE:-release}"
case "$mode" in
  debug) cargo build --locked ;;
  release) cargo build --release --locked ;;
  *) printf "Unknown build mode: %s\n" "$mode" >&2; exit 1 ;;
esac
app="build/flowGit.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "target/$mode/flowgit" "$app/Contents/MacOS/flowgit"
cp assets/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>flowgit</string>
<key>CFBundleIdentifier</key><string>com.blackprince001.flowgit</string>
<key>CFBundleName</key><string>flowGit</string>
<key>CFBundleDisplayName</key><string>flowGit</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
codesign --force --deep --sign - "$app"
printf 'Built %s\n' "$app"
