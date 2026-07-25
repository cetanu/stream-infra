#!/bin/bash
set -euo pipefail

DDNS_H="${DDNS_HOST:-@}"

if [ -z "${DDNS_DOMAIN:-}" ] || [ -z "${DDNS_PASSWORD:-}" ]; then
    echo "DDNS_DOMAIN and/or DDNS_PASSWORD not provided. Skipping DDNS update."
    exit 0
fi
echo "Updating Namecheap Dynamic DNS for ${DDNS_H}.${DDNS_DOMAIN}..."

IP=$(curl -s --max-time 5 http://169.254.169.254/v1/interfaces/0/ipv4/address || true)
IP_PARAM=""
if [ -n "$IP" ]; then
    IP_PARAM="&ip=${IP}"
fi

RESPONSE=$(curl -s "https://dynamicdns.park-your-domain.com/update?host=${DDNS_H}&domain=${DDNS_DOMAIN}&password=${DDNS_PASSWORD}${IP_PARAM}")

if echo "$RESPONSE" | grep -q "<ErrCount>0</ErrCount>"; then
    echo "Successfully updated Namecheap DDNS record for ${DDNS_H}.${DDNS_DOMAIN}."
else
    echo "Error updating Namecheap DDNS:" >&2
    echo "$RESPONSE" >&2
    exit 1
fi
