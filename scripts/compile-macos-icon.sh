#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
output=${1:?Usage: compile-macos-icon.sh OUTPUT_DIRECTORY}
mkdir -p "$output"
xcrun actool desktop/resources/AppIcon.icon \
  --compile "$output" \
  --platform macosx \
  --minimum-deployment-target 11.0 \
  --app-icon AppIcon \
  --output-partial-info-plist "$output/icon-info.plist" \
  --output-format human-readable-text
