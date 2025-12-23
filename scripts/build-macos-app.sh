#!/usr/bin/env bash
set -euo pipefail

# Builds a minimal menu-bar-only .app bundle for the tray binary.
# Output: dist/monitorctl.app

cd "$(dirname "$0")/.."

APP_NAME="monitorctl"
APP_ID="com.monitorctl.monitorctl"

mkdir -p dist

echo "Building Rust binary (release)..."
cargo build --release --bin monitortray

# Respect custom target dir if set; otherwise Cargo uses ./target.
target_dir="${CARGO_TARGET_DIR:-target}"
if [[ "${target_dir}" != /* ]]; then
  target_dir="$(pwd)/${target_dir}"
fi
src_bin="${target_dir}/release/monitortray"

if [[ ! -f "${src_bin}" ]]; then
  echo "ERROR: built binary not found at ${src_bin}" >&2
  exit 1
fi

app_dir="dist/${APP_NAME}.app"
contents="${app_dir}/Contents"
macos_dir="${contents}/MacOS"
resources_dir="${contents}/Resources"

rm -rf "${app_dir}"
mkdir -p "${macos_dir}" "${resources_dir}"

cp -f "${src_bin}" "${macos_dir}/${APP_NAME}"
chmod +x "${macos_dir}/${APP_NAME}"

version="$(sed -n 's/^version = \"\\(.*\\)\"$/\\1/p' Cargo.toml | head -n 1)"
if [[ -z "${version}" ]]; then
  version="0.0.0"
fi

cat > "${contents}/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>${APP_NAME}</string>
  <key>CFBundleExecutable</key><string>${APP_NAME}</string>
  <key>CFBundleIdentifier</key><string>${APP_ID}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>${APP_NAME}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

echo "Built ${app_dir}"
echo "Run: open \"${app_dir}\""
