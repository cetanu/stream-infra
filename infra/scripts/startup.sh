#!/bin/bash
set -euo pipefail

# Config content can be supplied via RTMP_CONFIG environment variable or $1 argument
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

# Write config.toml if config content is provided
printf '%s\n' "${CONFIG_CONTENT}" > "${CONFIG_PATH}"
chmod 600 "${CONFIG_PATH}"

# Download binary & install systemd service
curl -fsSL "${DOWNLOAD_URL}" -o "${BINARY_PATH}"
chmod +x "${BINARY_PATH}"
"${BINARY_PATH}" install-systemd \
    --work-dir "${TARGET_DIR}" \
    --config-path "${CONFIG_PATH}"
