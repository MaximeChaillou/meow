#!/bin/sh
# Packs target/release/Meow.app for a release, into dist/:
#   Meow.dmg      to install by hand (drag Meow to Applications)
#   Meow.zip      what the app downloads to update itself
#   version.json  the commit and the zip's SHA-256, read by the update check
# Usage: MEOW_COMMIT=<full hash> scripts/package.sh
set -eu

cd "$(dirname "$0")/.."
: "${MEOW_COMMIT:?set MEOW_COMMIT to the full commit hash}"

APP=target/release/Meow.app
DIST=dist
rm -rf "$DIST"
mkdir -p "$DIST"

ditto -c -k --sequesterRsrc --keepParent "$APP" "$DIST/Meow.zip"

STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname Meow -srcfolder "$STAGE" -ov -format UDZO "$DIST/Meow.dmg"
rm -rf "$STAGE"

SHA256=$(shasum -a 256 "$DIST/Meow.zip" | cut -d' ' -f1)
BUILT_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
cat > "$DIST/version.json" <<JSON
{"commit": "$MEOW_COMMIT", "sha256": "$SHA256", "built_at": "$BUILT_AT"}
JSON
echo "Packed $DIST: $(ls "$DIST" | tr '\n' ' ')"
