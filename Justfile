set shell := ["bash", "-c"]

default:
    @just --list

deploy:
    #!/usr/bin/env bash
    set -euo pipefail
    pushd infra
    pulumi up
    popd

configure-pillar file="pillar.sls":
    #!/usr/bin/env bash
    set -euo pipefail
    pushd infra
    pulumi config set pillar --secret "$(< ../{{file}})"
    popd
