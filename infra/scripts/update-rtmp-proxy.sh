#!/bin/bash
set -euo pipefail

readonly binary_path="/opt/rtmp-proxy/rtmp-proxy"
readonly release_url="https://github.com/cetanu/stream-infra/releases/latest/download/rtmp-proxy"
readonly checksum_url="${release_url}.sha256"

mkdir -p "$(dirname "${binary_path}")"
temporary_checksum="$(mktemp "${binary_path}.checksum.XXXXXX")"
trap 'rm -f "${temporary_checksum}"' EXIT

curl -fsSL --retry 3 --connect-timeout 10 \
    -H "Cache-Control: no-cache" \
    "${checksum_url}" -o "${temporary_checksum}"
expected_hash="$(awk 'NF { print $1; exit }' "${temporary_checksum}")"
if [[ ! "${expected_hash}" =~ ^[[:xdigit:]]{64}$ ]]; then
    echo "Release checksum is invalid" >&2
    exit 1
fi

if [[ -f "${binary_path}" ]] && [[ "$(sha256sum "${binary_path}" | awk '{print $1}')" == "${expected_hash}" ]]; then
    echo "RTMP proxy binary is already up to date."
    exit 0
fi

temporary_binary="$(mktemp "${binary_path}.new.XXXXXX")"
trap 'rm -f "${temporary_checksum}" "${temporary_binary}"' EXIT
curl -fsSL --retry 3 --connect-timeout 10 \
    -H "Cache-Control: no-cache" \
    "${release_url}" -o "${temporary_binary}"
if [[ "$(sha256sum "${temporary_binary}" | awk '{print $1}')" != "${expected_hash}" ]]; then
    echo "Downloaded RTMP proxy binary does not match its release checksum" >&2
    exit 1
fi
chmod 0755 "${temporary_binary}"
install -m 0755 "${temporary_binary}" "${binary_path}"
systemctl restart rtmp-proxy.service
echo "Updated and restarted the RTMP proxy."
