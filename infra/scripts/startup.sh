set -euo pipefail

CONFIG_CONTENT="${RTMP_CONFIG:-"${1:-""}"}"
REPO_SLUG="${GITHUB_REPOSITORY:-"cetanu/stream-infra"}"
TARGET_DIR="/opt/rtmp-proxy"
BINARY_PATH="${TARGET_DIR}/rtmp-proxy"
CONFIG_PATH="${TARGET_DIR}/config.toml"
DOWNLOAD_URL="https://github.com/${REPO_SLUG}/releases/latest/download/rtmp-proxy"

apt-get update
apt-get install -y ffmpeg curl ca-certificates

mkdir -p "${TARGET_DIR}"

if [ -z "${CONFIG_CONTENT}" ]; then
    echo "No RTMP configuration content was supplied." >&2
    echo "Please configure 'rtmpConfig' via Pulumi: pulumi config set rtmpConfig --secret \"\$(cat ../config.toml)\"" >&2
    exit 1
fi

printf '%b\n' "${CONFIG_CONTENT}" > "${CONFIG_PATH}"
chmod 600 "${CONFIG_PATH}"

curl -fsSL "${DOWNLOAD_URL}" -o "${BINARY_PATH}"
chmod +x "${BINARY_PATH}"
"${BINARY_PATH}" install-systemd \
    --work-dir "${TARGET_DIR}" \
    --config-path "${CONFIG_PATH}"

ufw allow 1935/tcp || true
