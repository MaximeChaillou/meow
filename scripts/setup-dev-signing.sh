#!/bin/sh
# Creates the self-signed "Meow Dev" code-signing identity in the login
# keychain, once. Builds signed with it (see scripts/dev-run.sh) keep their
# Keychain access from one build to the next.
set -eu

IDENTITY="Meow Dev"

if security find-identity -p codesigning 2>/dev/null | grep -q "\"$IDENTITY\""; then
    echo "\"$IDENTITY\" already exists."
    exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cat > "$work/cert.cnf" <<CONF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $IDENTITY
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
CONF

# The system's LibreSSL writes a .p12 that `security import` reads.
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
    -config "$work/cert.cnf" -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
password=$(/usr/bin/openssl rand -hex 16)
/usr/bin/openssl pkcs12 -export -name "$IDENTITY" -passout "pass:$password" \
    -inkey "$work/key.pem" -in "$work/cert.pem" -out "$work/identity.p12"
security import "$work/identity.p12" -k "$HOME/Library/Keychains/login.keychain-db" \
    -P "$password" -T /usr/bin/codesign >/dev/null

echo "Created \"$IDENTITY\". The next run asks for Keychain access once: choose Always Allow."
