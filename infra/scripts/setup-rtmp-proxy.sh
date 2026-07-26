#!/bin/bash
set -euo pipefail

readonly work_dir="/opt/rtmp-proxy"
readonly state_dir="/var/lib/rtmp-proxy"
readonly bootstrap_config="/run/rtmp-proxy-bootstrap/config.toml"
readonly persistent_config="${state_dir}/config.toml"
readonly persistent_database="${state_dir}/config.sqlite3"
readonly release_url="https://github.com/cetanu/stream-infra/releases/latest/download/rtmp-proxy"
readonly checksum_url="${release_url}.sha256"

mkdir -p "${work_dir}" "${state_dir}"
chmod 0700 "${state_dir}"

temporary_checksum="$(mktemp)"
temporary_binary="$(mktemp)"
trap 'rm -f "${temporary_checksum}" "${temporary_binary}"' EXIT
curl -fsSL -H "Cache-Control: no-cache" "${checksum_url}" -o "${temporary_checksum}"
expected_hash="$(awk 'NF { print $1; exit }' "${temporary_checksum}")"
if [[ ! "${expected_hash}" =~ ^[[:xdigit:]]{64}$ ]]; then
    echo "Release checksum is invalid" >&2
    exit 1
fi
curl -fsSL -H "Cache-Control: no-cache" "${release_url}" -o "${temporary_binary}"
if [[ "$(sha256sum "${temporary_binary}" | awk '{print $1}')" != "${expected_hash}" ]]; then
    echo "Downloaded RTMP proxy binary does not match its release checksum" >&2
    exit 1
fi
install -m 0755 "${temporary_binary}" "${work_dir}/rtmp-proxy"

# Keep the host firewall aligned with the Vultr firewall group. The Vultr
# firewall still provides the source-IP restrictions for RTMP and HTTPS.
ufw allow 1935/tcp || true
ufw allow 80/tcp || true
ufw allow 443/tcp || true

root_source="$(findmnt -n -o SOURCE /)"
root_disk="$(lsblk -no PKNAME "${root_source}" | head -n 1)"
if [[ -z "${root_disk}" ]]; then
    root_disk="$(basename "${root_source}")"
fi

state_device=""
for _attempt in $(seq 1 120); do
    mapfile -t candidate_disks < <(
        lsblk -dn -o NAME,TYPE |
            awk -v root="${root_disk}" '$2 == "disk" && $1 != root { print "/dev/" $1 }'
    )
    if [[ "${#candidate_disks[@]}" -eq 1 ]]; then
        state_device="${candidate_disks[0]}"
        break
    fi
    if [[ "${#candidate_disks[@]}" -gt 1 ]]; then
        echo "Refusing to choose between multiple non-root disks: ${candidate_disks[*]}" >&2
        exit 1
    fi
    sleep 5
done

if [[ -z "${state_device}" ]]; then
    echo "Timed out waiting for the Pulumi-managed state volume" >&2
    exit 1
fi

filesystem_type="$(blkid -p -s TYPE -o value "${state_device}" || true)"
if [[ -z "${filesystem_type}" ]]; then
    mkfs.ext4 -L rtmp-proxy-state "${state_device}"
elif [[ "${filesystem_type}" != "ext4" ]]; then
    echo "Refusing state volume with unexpected filesystem '${filesystem_type}'" >&2
    exit 1
fi

volume_uuid="$(blkid -s UUID -o value "${state_device}")"
if ! grep -q "UUID=${volume_uuid}" /etc/fstab; then
    echo "UUID=${volume_uuid} ${state_dir} ext4 defaults,nofail 0 2" >> /etc/fstab
fi
mountpoint -q "${state_dir}" || mount "${state_dir}"
chmod 0700 "${state_dir}"

if [[ ! -e "${persistent_database}" && ! -e "${persistent_config}" ]]; then
    install -m 0600 "${bootstrap_config}" "${persistent_config}"
fi

"${work_dir}/rtmp-proxy" install-systemd \
    --work-dir "${work_dir}" \
    --config-path "${persistent_config}"
