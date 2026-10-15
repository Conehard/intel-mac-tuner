#!/bin/sh
# Build "Intel Mac Tuner.app" without Xcode: just cargo + the Command Line Tools.
# Usage: scripts/make-app.sh [dest-folder]   (default: ~/Applications)
set -eu

cd "$(dirname "$0")/.."
DEST="${1:-$HOME/Applications}"
NAME="Intel Mac Tuner"
BIN="intel-mac-tuner-menu"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"

cargo build --release --locked -p menubar

APP="target/$NAME.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/release/$BIN" "$APP/Contents/MacOS/$BIN"

# App icon: build AppIcon.icns from the iconset when iconutil is available.
ICON_KEY=""
if [ -d assets/AppIcon.iconset ] && command -v iconutil >/dev/null 2>&1; then
    iconutil -c icns assets/AppIcon.iconset -o "$APP/Contents/Resources/AppIcon.icns"
    ICON_KEY="    <key>CFBundleIconFile</key>
    <string>AppIcon</string>"
fi

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$NAME</string>
    <key>CFBundleIdentifier</key>
    <string>dev.intelmactuner.menu</string>
    <key>CFBundleExecutable</key>
    <string>$BIN</string>
$ICON_KEY
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.13</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

# Ad-hoc (local) signature so Gatekeeper does not complain about a modified binary.
codesign --force --sign - "$APP" >/dev/null 2>&1 || echo "warning: codesign unavailable; the app still runs"

mkdir -p "$DEST"
# Close an old instance before replacing it.
pkill -x "$BIN" 2>/dev/null || true
rm -rf "$DEST/$NAME.app"
cp -R "$APP" "$DEST/"

echo "Done: $DEST/$NAME.app"
echo "Open now: open \"$DEST/$NAME.app\""
