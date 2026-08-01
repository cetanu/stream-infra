#!/bin/bash
set -euo pipefail

readonly key_dir="/etc/salt/gpgkeys"
readonly key_file="/run/deployment-gpg-private-key.asc"

cleanup() {
	rm -f "${key_file}"
}
trap cleanup EXIT

if [[ ! -s "${key_file}" ]]; then
	echo "Salt GPG private key is missing or empty" >&2
	exit 1
fi

install -d -o root -g root -m 0700 "${key_dir}"
gpg --batch --homedir "${key_dir}" --import "${key_file}"

secret_keys="$(gpg --batch --homedir "${key_dir}" --with-colons --list-secret-keys)"
if ! grep -q '^sec:' <<<"${secret_keys}"; then
	echo "Salt GPG keyring contains no secret key after import" >&2
	exit 1
fi
