#!/bin/bash
set -euo pipefail

readonly work_dir="/opt/rtmp-proxy"
readonly bootstrap_config="/run/rtmp-proxy-bootstrap/config.toml"
readonly config_path="${work_dir}/config.toml"
readonly database_path="${work_dir}/config.sqlite3"
readonly release_url="https://github.com/cetanu/stream-infra/releases/latest/download/rtmp-proxy"
readonly checksum_url="${release_url}.sha256"

mkdir -p "${work_dir}"
chmod 0700 "${work_dir}"

if [[ ! -e "${database_path}" && ! -e "${config_path}" ]]; then
    install -m 0600 "${bootstrap_config}" "${config_path}"
fi

temporary_checksum="$(mktemp)"
temporary_binary="$(mktemp)"
trap 'rm -f "${temporary_checksum}" "${temporary_binary}"' EXIT
curl -fsSL --retry 3 --connect-timeout 10 --max-time 120 \
    -H "Cache-Control: no-cache" "${checksum_url}" -o "${temporary_checksum}"
expected_hash="$(awk 'NF { print $1; exit }' "${temporary_checksum}")"
if [[ ! "${expected_hash}" =~ ^[[:xdigit:]]{64}$ ]]; then
    echo "Release checksum is invalid" >&2
    exit 1
fi
curl -fsSL --retry 3 --connect-timeout 10 --max-time 120 \
    -H "Cache-Control: no-cache" "${release_url}" -o "${temporary_binary}"
if [[ "$(sha256sum "${temporary_binary}" | awk '{print $1}')" != "${expected_hash}" ]]; then
    echo "Downloaded RTMP proxy binary does not match its release checksum" >&2
    exit 1
fi
install -m 0755 "${temporary_binary}" "${work_dir}/rtmp-proxy"

"${work_dir}/rtmp-proxy" install-systemd \
    --work-dir "${work_dir}" \
    --config-path "${config_path}"
