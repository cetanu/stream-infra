#!/bin/bash
set -euo pipefail

mkdir -p /opt/rtmp-proxy
curl -fsSL https://github.com/cetanu/stream-infra/releases/latest/download/rtmp-proxy -o /opt/rtmp-proxy/rtmp-proxy
chmod +x /opt/rtmp-proxy/rtmp-proxy
/opt/rtmp-proxy/rtmp-proxy install-systemd --work-dir /opt/rtmp-proxy --config-path /opt/rtmp-proxy/config.toml
ufw allow 1935/tcp || true
ufw allow 80/tcp || true
ufw allow 443/tcp || true
