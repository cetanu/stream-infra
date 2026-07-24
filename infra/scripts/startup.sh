#!/bin/bash
set -euo pipefail

# Config content MUST be supplied via RTMP_CONFIG environment variable or $1 argument
CONFIG_CONTENT="${RTMP_CONFIG:-"${1:-""}"}"
REPO_SLUG="${GITHUB_REPOSITORY:-"vsyrakis/stream-infra"}"
TARGET_DIR="/opt/rtmp-proxy"
BINARY_PATH="${TARGET_DIR}/rtmp-proxy"
CONFIG_PATH="${TARGET_DIR}/config.toml"
DOWNLOAD_URL="https://github.com/${REPO_SLUG}/releases/latest/download/rtmp-proxy"

# Update OS packages and install runtime dependencies
apt-get update
apt-get install -y ffmpeg curl ca-certificates

# Ensure target runtime directory exists
mkdir -p "${TARGET_DIR}"

# Fail-Closed Check: Error out if no config.toml content is supplied
if [ -z "${CONFIG_CONTENT}" ]; then
    echo "No RTMP configuration content was supplied." >&2
    echo "Please configure 'rtmpConfig' via Pulumi: pulumi config set rtmpConfig --secret \"\$(cat ../config.toml)\"" >&2
    exit 1
fi

printf '%s\n' "${CONFIG_CONTENT}" > "${CONFIG_PATH}"
chmod 600 "${CONFIG_PATH}"

# Download binary & install systemd service
curl -fsSL "${DOWNLOAD_URL}" -o "${BINARY_PATH}"
chmod +x "${BINARY_PATH}"
"${BINARY_PATH}" install-systemd \
    --work-dir "${TARGET_DIR}" \
    --config-path "${CONFIG_PATH}"
