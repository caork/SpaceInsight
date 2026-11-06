#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
profile=${1:-release}
case "$profile" in
  release) cargo build --release --locked -p spaceinsight-desktop ;;
  debug) cargo build --locked -p spaceinsight-desktop ;;
  *) echo "Usage: $0 [release|debug]" >&2; exit 2 ;;
esac
bundle=target/$profile/SpaceInsight.app
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "target/$profile/spaceinsight" "$bundle/Contents/MacOS/SpaceInsight"
# Remove the former static icon when rebuilding an existing local bundle.
rm -f "$bundle/Contents/Resources/icon.icns"
# Compile the Icon Composer source rather than flattening its appearances into
# an ICNS. macOS selects the light/dark/clear/tinted rendition from Assets.car.
icon_build=$(mktemp -d -t spaceinsight-icon)
trap 'rm -rf "$icon_build"' EXIT HUP INT TERM
# Icon catalogs are architecture-independent. CI compiles them on modern macOS
# and reuses them for Intel runners whose older asset runtime cannot render .icon.
if [ -n "${SPACEINSIGHT_ICON_DIR:-}" ]; then
  icon_source=$SPACEINSIGHT_ICON_DIR
else
  ./scripts/compile-macos-icon.sh "$icon_build"
  icon_source=$icon_build
fi
cp "$icon_source/Assets.car" "$bundle/Contents/Resources/Assets.car"
cp "$icon_source/AppIcon.icns" "$bundle/Contents/Resources/AppIcon.icns"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>SpaceInsight</string>
<key>CFBundleDisplayName</key><string>SpaceInsight</string>
<key>CFBundleExecutable</key><string>SpaceInsight</string>
<key>CFBundleIdentifier</key><string>com.hawkingrad.spaceinsight</string>
<key>CFBundleVersion</key><string>0.3.1</string>
<key>CFBundleShortVersionString</key><string>0.3.0</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
</dict></plist>
PLIST
# Keep the compiler's icon metadata (including CFBundleIconName) in sync with
# the compiled catalog. actool also emits AppIcon.icns for older macOS.
/usr/libexec/PlistBuddy -c "Merge $icon_source/icon-info.plist" "$bundle/Contents/Info.plist"
codesign --force --deep --sign - "$bundle"
echo "Built $bundle (ad-hoc signed; distribution requires Developer ID signing and notarization)."
