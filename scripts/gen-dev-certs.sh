#!/usr/bin/env bash
# Generate development certificates for the gateway's mutual-TLS auth.
#
# Idempotent: existing certificates are reused unless --force is passed.
#
# Output (all under .certs/, gitignored):
#   ca.pem          development CA (self-signed)
#   server.pem      gateway server certificate
#                   (SAN: localhost, 127.0.0.1, ibkr-gateway)
#   server-key.pem  gateway server private key
#   client.pem      client certificate (SAN: ibkr-cli)
#   client-key.pem  client private key
#
# The compose `ibkr-gateway` service mounts this directory at /certs and uses
# server.pem / server-key.pem / ca.pem; `ibkr` uses client.pem /
# client-key.pem / ca.pem (see .env.example).

set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${CERT_DIR:-$DIR/.certs}"
FORCE=0
if [[ "${1:-}" == "--force" ]]; then
    FORCE=1
fi

mkdir -p "$OUT"

if [[ $FORCE -eq 0 ]] && [[ -f "$OUT/server.pem" && -f "$OUT/server-key.pem" && -f "$OUT/ca.pem" ]]; then
    echo ".> certificates already exist in $OUT (use --force to regenerate)"
    exit 0
fi

echo ".> generating development CA and certificates in $OUT"

# Development CA
openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$OUT/ca-key.pem" -out "$OUT/ca.pem" \
    -days 3650 -subj "/CN=IBKR Gateway Dev CA" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null

# Server certificate (mTLS server side). `localhost`/127.0.0.1 cover a gateway
# on the host; `ibkr-gateway` covers the compose service name.
openssl req -newkey rsa:2048 -nodes \
    -keyout "$OUT/server-key.pem" -out "$OUT/server.csr" \
    -subj "/CN=ibkr-gateway" 2>/dev/null
openssl x509 -req -in "$OUT/server.csr" -CA "$OUT/ca.pem" -CAkey "$OUT/ca-key.pem" \
    -CAcreateserial -out "$OUT/server.pem" -days 825 \
    -extfile <(printf "subjectAltName=DNS:localhost,IP:127.0.0.1,DNS:ibkr-gateway\nextendedKeyUsage=serverAuth") \
    2>/dev/null

# Client certificate (this client). The gateway pins the signing CA, not the
# subject, so the exact CN only shows up in gateway logs.
openssl req -newkey rsa:2048 -nodes \
    -keyout "$OUT/client-key.pem" -out "$OUT/client.csr" \
    -subj "/CN=ibkr-cli" 2>/dev/null
openssl x509 -req -in "$OUT/client.csr" -CA "$OUT/ca.pem" -CAkey "$OUT/ca-key.pem" \
    -CAcreateserial -out "$OUT/client.pem" -days 825 \
    -extfile <(printf "subjectAltName=DNS:ibkr-cli\nextendedKeyUsage=clientAuth") \
    2>/dev/null

rm -f "$OUT/server.csr" "$OUT/client.csr"
chmod 600 "$OUT"/*-key.pem

echo ".> done:"
echo "    CA:          $OUT/ca.pem"
echo "    Server cert: $OUT/server.pem"
echo "    Server key:  $OUT/server-key.pem"
echo "    Client cert: $OUT/client.pem"
echo "    Client key:  $OUT/client-key.pem"
