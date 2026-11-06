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
cp desktop/resources/icon.icns "$bundle/Contents/Resources/icon.icns"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>SpaceInsight</string>
<key>CFBundleDisplayName</key><string>SpaceInsight</string>
<key>CFBundleExecutable</key><string>SpaceInsight</string>
<key>CFBundleIdentifier</key><string>com.hawkingrad.spaceinsight</string>
<key>CFBundleVersion</key><string>0.3.0</string>
<key>CFBundleShortVersionString</key><string>0.3.0</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>icon.icns</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
</dict></plist>
PLIST
codesign --force --deep --sign - "$bundle"
echo "Built $bundle (ad-hoc signed; distribution requires Developer ID signing and notarization)."
