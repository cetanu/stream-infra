set shell := ["bash", "-c"]

default:
    @just --list

deploy:
    #!/usr/bin/env bash
    set -euo pipefail
    pushd infra
    pulumi up
    popd

configure-gpg-key file:
    #!/usr/bin/env bash
    set -euo pipefail
    pushd infra
    pulumi config set gpgPrivateKey --secret "$(< ../{{file}})"
    popd
