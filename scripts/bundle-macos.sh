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
icon_metadata=$(mktemp -t spaceinsight-icon)
trap 'rm -f "$icon_metadata"' EXIT HUP INT TERM
xcrun actool desktop/resources/AppIcon.icon \
  --compile "$bundle/Contents/Resources" \
  --platform macosx \
  --minimum-deployment-target 11.0 \
  --app-icon AppIcon \
  --output-partial-info-plist "$icon_metadata" \
  --output-format human-readable-text
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
/usr/libexec/PlistBuddy -c "Merge $icon_metadata" "$bundle/Contents/Info.plist"
codesign --force --deep --sign - "$bundle"
echo "Built $bundle (ad-hoc signed; distribution requires Developer ID signing and notarization)."
