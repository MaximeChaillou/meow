#!/bin/sh
# Cargo runner (see .cargo/config.toml): signs the binary with the "Meow Dev"
# identity, then runs it. The Keychain trusts an app by its signature, and an
# ad-hoc signature changes with every build; this one does not.
set -eu

binary="$1"
shift

IDENTITY="Meow Dev"
IDENTIFIER="dev.meow.cat"

if security find-identity -p codesigning 2>/dev/null | grep -q "\"$IDENTITY\""; then
    codesign --force --sign "$IDENTITY" --identifier "$IDENTIFIER" "$binary" 2>/dev/null ||
        echo "meow: could not sign $binary; the Keychain may ask for access" >&2
fi

exec "$binary" "$@"
