# Justfile for stream-infra

set shell := ["bash", "-c"]

# Default recipe: print available commands
default:
    @just --list

# Stream a synthetic 720p 30fps video/audio test pattern via FFmpeg
# Usage: just test-stream <VULTR_IP> [STREAM_KEY]
test-stream ip key="teststream":
    ffmpeg -re -f lavfi -i testsrc=size=1280x720:rate=30 -f lavfi -i sine=frequency=1000 -c:v libx264 -c:a aac -f flv "rtmp://{{ip}}:1935/live/{{key}}"

# Stream a local video file via FFmpeg
# Usage: just test-file <VULTR_IP> <PATH_TO_MP4> [STREAM_KEY]
test-file ip file key="teststream":
    ffmpeg -re -i "{{file}}" -c copy -f flv "rtmp://{{ip}}:1935/live/{{key}}"

# Build the Rust RTMP multiplexer binary
build:
    cargo build

# Run cargo check for Rust code
check:
    cargo check

# Run the RTMP multiplexer locally
run:
    cargo run

update-conf:
    #!/usr/bin/env bash
    nvim config.toml
    pushd infra
    pulumi config set rtmpConfig --secret "$(< ../config.toml)"
    popd

# Deploy infrastructure to Vultr via Pulumi (syncs latest config.toml)
deploy: update-conf
    #!/usr/bin/env bash
    pushd infra
    pulumi up
    popd
