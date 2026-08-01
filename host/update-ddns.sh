#!/bin/bash
set -euo pipefail

metadata_url="http://169.254.169.254/v1/interfaces/0/ipv4/address"
update_url="https://dynamicdns.park-your-domain.com/update"

ip=$(curl --fail --silent --show-error --max-time 5 "$metadata_url")
response=$(curl --fail --silent --show-error --get "$update_url" \
    --data-urlencode "host=${DDNS_HOST}" \
    --data-urlencode "domain=${DDNS_DOMAIN}" \
    --data-urlencode "password=${DDNS_PASSWORD}" \
    --data-urlencode "ip=${ip}")

if ! grep -q '<ErrCount>0</ErrCount>' <<<"$response"; then
    echo "Namecheap DDNS update failed for ${DDNS_HOST}.${DDNS_DOMAIN}" >&2
    echo "$response" >&2
    exit 1
fi

echo "Updated ${DDNS_HOST}.${DDNS_DOMAIN} to ${ip}"
