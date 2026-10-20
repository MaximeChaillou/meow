#!/bin/sh
# Builds target/release/Meow.app (no Dock icon, starts the walking cat).
# Usage: scripts/bundle.sh [--universal] [--install]
#   --universal  one binary for Apple silicon and Intel (needs both rustup targets)
#   --install    copies it to ~/Applications
# MEOW_COMMIT (the commit hash, set by the release workflow) makes a build that
# updates itself from the GitHub releases; without it the app never updates.
set -eu

cd "$(dirname "$0")/.."

UNIVERSAL=false
INSTALL=false
for arg in "$@"; do
    case "$arg" in
        --universal) UNIVERSAL=true ;;
        --install) INSTALL=true ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

APP=target/release/Meow.app
BINARY=target/release/meow
if $UNIVERSAL; then
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    mkdir -p target/release
    lipo -create -output "$BINARY" \
        target/aarch64-apple-darwin/release/meow \
        target/x86_64-apple-darwin/release/meow
else
    cargo build --release
fi

# The short commit as the build number, so Finder's "Get Info" shows it.
BUILD=$(printf '%s' "${MEOW_COMMIT:-local}" | cut -c1-7)

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$BINARY" "$APP/Contents/MacOS/meow"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Meow</string>
    <key>CFBundleIdentifier</key><string>dev.meow.cat</string>
    <key>CFBundleExecutable</key><string>meow</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>CFBundleVersion</key><string>$BUILD</string>
    <key>LSUIElement</key><true/>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
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

if $INSTALL; then
    mkdir -p "$HOME/Applications"
    rm -rf "$HOME/Applications/Meow.app"
    cp -R "$APP" "$HOME/Applications/"
    echo "Installed to ~/Applications/Meow.app"
fi
