#!/bin/zsh
# Builds a double-clickable app at target/release/Bildesveip.app.
set -e
cd "$(dirname "$0")/.."
cargo build --release
APP="target/release/Bildesveip.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/bildesveip "$APP/Contents/MacOS/"
cp assets/icon.icns "$APP/Contents/Resources/"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Bildesveip</string>
<key>CFBundleIdentifier</key><string>io.github.sti90.bildesveip</string>
<key>CFBundleExecutable</key><string>bildesveip</string>
<key>CFBundleIconFile</key><string>icon</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>NSPhotoLibraryUsageDescription</key><string>Bildesveip viser bildene dine så du kan sortere dem, og flytter bildene du velger til Nylig slettet.</string>
</dict></plist>
PLIST
codesign --force --sign - "$APP"
echo "$PWD/$APP"
