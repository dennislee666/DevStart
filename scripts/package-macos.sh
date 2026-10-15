#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != "Darwin" ]]; then
  printf '%s\n' 'macOS packaging requires macOS.' >&2
  exit 1
fi
deployment_target="${MACOSX_DEPLOYMENT_TARGET:-12.0}"
export MACOSX_DEPLOYMENT_TARGET="$deployment_target"
macos_arch="${DEVSTART_MACOS_ARCH:-universal}"
package_binary="target/release/dev-start"
case "$macos_arch" in
  universal)
    if ! command -v rustup >/dev/null 2>&1; then
      printf '%s\n' 'Universal macOS packaging requires rustup. Set DEVSTART_MACOS_ARCH=native for a host-only build.' >&2
      exit 1
    fi
    toolchain="${RUSTUP_TOOLCHAIN:-$(rustup show active-toolchain | awk '{print $1}')}"
    rustc_bin=$(rustup which --toolchain "$toolchain" rustc)
    cargo_bin=$(rustup which --toolchain "$toolchain" cargo)
    installed=$(rustup target list --installed --toolchain "$toolchain")
    missing=()
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
      if ! grep -qx "$target" <<<"$installed"; then missing+=("$target"); fi
    done
    if ((${#missing[@]})); then
      RUSTUP_DIST_SERVER="${RUSTUP_DIST_SERVER:-https://rsproxy.cn}" \
      RUSTUP_UPDATE_ROOT="${RUSTUP_UPDATE_ROOT:-https://rsproxy.cn/rustup}" \
        rustup target add --toolchain "$toolchain" "${missing[@]}"
    fi
    RUSTC="$rustc_bin" "$cargo_bin" build --release --locked --target aarch64-apple-darwin "$@"
    RUSTC="$rustc_bin" "$cargo_bin" build --release --locked --target x86_64-apple-darwin "$@"
    host_arch=$(uname -m)
    case "$host_arch" in
      arm64) host_binary='target/aarch64-apple-darwin/release/dev-start' ;;
      x86_64) host_binary='target/x86_64-apple-darwin/release/dev-start' ;;
      *) printf 'Unsupported macOS host architecture: %s\n' "$host_arch" >&2; exit 1 ;;
    esac
    # Both slices use the same Cargo manifest and feature flags. Read edition from
    # the host slice so packaging never requires Rosetta on an Apple Silicon Mac.
    edition=$("$host_binary" --edition)
    mkdir -p target/universal-apple-darwin/release
    package_binary='target/universal-apple-darwin/release/dev-start'
    lipo -create target/aarch64-apple-darwin/release/dev-start target/x86_64-apple-darwin/release/dev-start -output "$package_binary.next"
    mv -f "$package_binary.next" "$package_binary"
    ;;
  native)
    cargo build --release --locked "$@"
    edition=$(target/release/dev-start --edition)
    ;;
  *) printf 'DEVSTART_MACOS_ARCH must be universal or native, got: %s\n' "$macos_arch" >&2; exit 2 ;;
esac
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
cp "$package_binary" "$app/Contents/MacOS/dev-start.next"
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
