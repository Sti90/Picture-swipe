#!/bin/zsh
# Builds a drag-to-install disk image at target/release/Bildesveip-<version>.dmg.
set -e
cd "$(dirname "$0")/.."
./scripts/bundle.sh
APP="target/release/Bildesveip.app"
VERSION=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$APP/Contents/Info.plist")
DMG="target/release/Bildesveip-$VERSION.dmg"

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/Bildesveip.app"
ln -s /Applications "$STAGE/Applications"

hdiutil create -ov -quiet -fs APFS -format UDZO -volname "Bildesveip" -srcfolder "$STAGE" "$DMG"
hdiutil verify -quiet "$DMG"
echo "$PWD/$DMG"
