#!/bin/sh
# Builds target/release/Meow.app (no Dock icon, starts the walking cat).
# Usage: scripts/bundle.sh [--install]   (--install copies it to ~/Applications)
set -eu

cd "$(dirname "$0")/.."
cargo build --release

VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
# What the app is built from, for its updates (src/update.rs): it follows
# main from this checkout's origin, unless built with uncommitted changes.
COMMIT=$(git rev-parse HEAD 2>/dev/null || true)
REMOTE=$(git remote get-url origin 2>/dev/null | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' || true)
if [ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ]; then
    MODIFIED=true
else
    MODIFIED=false
fi

APP=target/release/Meow.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/meow "$APP/Contents/MacOS/meow"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Meow</string>
    <key>CFBundleIdentifier</key><string>dev.meow.cat</string>
    <key>CFBundleExecutable</key><string>meow</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>LSUIElement</key><true/>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>MeowCommit</key><string>$COMMIT</string>
    <key>MeowRemote</key><string>$REMOTE</string>
    <key>MeowModified</key><$MODIFIED/>
</dict>
</plist>
PLIST
# The "Meow Dev" identity (scripts/setup-dev-signing.sh) keeps the Keychain
# access granted to dev builds; without it, sign ad hoc.
if security find-identity -p codesigning 2>/dev/null | grep -q '"Meow Dev"'; then
    codesign --force --sign "Meow Dev" "$APP" >/dev/null 2>&1 || true
else
    codesign --force --sign - "$APP" >/dev/null 2>&1 || true
fi
echo "Built $APP"

if [ "${1:-}" = "--install" ]; then
    mkdir -p "$HOME/Applications"
    rm -rf "$HOME/Applications/Meow.app"
    cp -R "$APP" "$HOME/Applications/"
    echo "Installed to ~/Applications/Meow.app"
fi
