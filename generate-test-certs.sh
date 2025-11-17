#!/bin/bash

set -e

echo "🔒 Generating test TLS certificates for api.platform.network"
echo "⚠️  WARNING: These are SELF-SIGNED certificates for TESTING ONLY!"
echo ""

# Create certs directory
mkdir -p certs

# Generate private key
echo "📝 Generating private key..."
openssl genrsa -out certs/privkey.pem 2048

# Generate certificate signing request
echo "📝 Generating certificate signing request..."
openssl req -new -key certs/privkey.pem -out certs/cert.csr \
  -subj "/C=US/ST=State/L=City/O=Platform Network/CN=api.platform.network"

# Generate self-signed certificate (valid for 365 days)
echo "📝 Generating self-signed certificate..."
openssl x509 -req -days 365 -in certs/cert.csr \
  -signkey certs/privkey.pem -out certs/fullchain.pem \
  -extfile <(printf "subjectAltName=DNS:api.platform.network,DNS:*.platform.network")

# Clean up CSR
rm certs/cert.csr

# Set proper permissions
chmod 600 certs/privkey.pem
chmod 644 certs/fullchain.pem

echo ""
echo "✅ Test certificates generated successfully!"
echo "   📄 certs/fullchain.pem (certificate)"
echo "   🔑 certs/privkey.pem (private key - permissions: 600)"
echo ""
echo "🚀 Start the production stack with:"
echo "   docker-compose -f docker-compose.production.yml up -d"
echo ""
echo "🌐 Access the API at:"
echo "   https://api.platform.network"
echo ""
echo "⚠️  IMPORTANT: Self-signed certificates will show security warnings in browsers."
echo "    For production, use Let's Encrypt certificates (see certs/README.md)"
echo ""

