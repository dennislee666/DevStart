#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != "Darwin" ]]; then
  printf '%s\n' 'macOS packaging requires macOS.' >&2
  exit 1
fi
cargo build --release --locked "$@"
edition=$(target/release/dev-start --edition)
bundle_id="com.devstart.public"
if [[ "$edition" == "personal" ]]; then
  bundle_id="com.devstart.desktop"
fi
app='dist/DevStart.app'
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp LICENSE "$app/Contents/Resources/LICENSE"
licenses='docs/third-party-licenses'
if [[ ! -d "$licenses" ]]; then
  licenses='docs/open-source/third-party-licenses'
fi
if [[ -d "$licenses" ]]; then
  mkdir -p "$app/Contents/Resources/third-party-licenses"
  cp -R "$licenses/." "$app/Contents/Resources/third-party-licenses/"
fi
iconset='target/devstart.iconset' 
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" assets/devstart-icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" assets/devstart-icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/DevStart.icns"
cp target/release/dev-start "$app/Contents/MacOS/dev-start.next"
mv -f "$app/Contents/MacOS/dev-start.next" "$app/Contents/MacOS/dev-start"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>DevStart</string>
<key>CFBundleDisplayName</key><string>DevStart</string>
<key>CFBundleIdentifier</key><string>$bundle_id</string>
<key>CFBundleExecutable</key><string>dev-start</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>2</string>
<key>CFBundleIconFile</key><string>DevStart.icns</string>
<key>NSHighResolutionCapable</key><true/>
<key>LSMinimumSystemVersion</key><string>12.0</string>
</dict></plist>
PLIST
codesign --force --sign - "$app"
# Update an existing legacy bundle in place so pinned Dock shortcuts also receive the next launch.
legacy='dist/Dev Start.app'
if [[ "$edition" == "personal" && -d "$legacy" && ! -L "$legacy" ]]; then
  mkdir -p "$legacy/Contents/Resources"
  cp "$app/Contents/Resources/DevStart.icns" "$legacy/Contents/Resources/DevStart.icns"
  cp "$app/Contents/Info.plist" "$legacy/Contents/Info.plist"
  cp "$app/Contents/Resources/LICENSE" "$legacy/Contents/Resources/LICENSE"
  if [[ -d "$app/Contents/Resources/third-party-licenses" ]]; then
    mkdir -p "$legacy/Contents/Resources/third-party-licenses"
    cp -R "$app/Contents/Resources/third-party-licenses/." "$legacy/Contents/Resources/third-party-licenses/"
  fi
  cp "$app/Contents/MacOS/dev-start" "$legacy/Contents/MacOS/dev-start.next"
  mv -f "$legacy/Contents/MacOS/dev-start.next" "$legacy/Contents/MacOS/dev-start"
  codesign --force --sign - "$legacy"
fi
/usr/libexec/PlistBuddy -c 'Print CFBundleExecutable' "$app/Contents/Info.plist"
printf 'Built %s\n' "$PWD/$app"
