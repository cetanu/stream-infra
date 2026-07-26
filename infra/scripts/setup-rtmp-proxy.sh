#!/bin/bash
set -euo pipefail

readonly work_dir="/opt/rtmp-proxy"
readonly state_dir="/var/lib/rtmp-proxy"
readonly bootstrap_config="/run/rtmp-proxy-bootstrap/config.toml"
readonly persistent_config="${state_dir}/config.toml"
readonly persistent_database="${state_dir}/config.sqlite3"

mkdir -p "${work_dir}" "${state_dir}"
chmod 0700 "${state_dir}"

curl -fsSL -H "Cache-Control: no-cache" \
    https://github.com/cetanu/stream-infra/releases/latest/download/rtmp-proxy \
    -o "${work_dir}/rtmp-proxy"
chmod 0755 "${work_dir}/rtmp-proxy"

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
