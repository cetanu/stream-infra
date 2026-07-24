# stream-infra

RTMP Stream Multiplexer powered by `rtmp-rs` and provisioned on Vultr with Pulumi Go SDK.

## Repository Structure

```
.
├── Cargo.toml           # Rust multiplexer dependencies (rtmp-rs, tokio, clap, toml)
├── config.example.toml  # Template for RTMP target destinations (YouTube, Twitch, X)
├── src/
│   └── main.rs          # RTMP proxy & multiplexer implementation using rtmp-rs
├── systemd/
│   └── rtmp-proxy.service  # Systemd service unit template embedded into Rust binary
└── infra/               # Pulumi infrastructure definition (Vultr Go SDK)
    ├── Pulumi.yaml
    ├── Pulumi.dev.yaml
    ├── constants.go     # Vultr OS IDs, Plans, and Region constants
    ├── main.go          # Pulumi stack with locked-down firewall & embedded UserData
    └── scripts/
        └── startup.sh   # Standalone Cloud-Init bash script embedded via //go:embed
```

## Security & Firewall Defaults

- **SSH (22)**: Disabled
- **Health Check (8080)**: Closed to public internet
- **RTMP (1935)**: Restricted to specified `allowedIngressIp`

## Deployment & Configuration Workflow

### 1. Configure Local Targets

Create `config.toml` from the example template:
```bash
cp config.example.toml config.toml
# Edit config.toml with your YouTube, Twitch, and X stream keys
```

### 2. Deploy to Vultr with Pulumi

Set your Vultr API Key, allowed IP, and encrypted `config.toml` secret:

```bash
cd infra
pulumi config set vultr:apiKey <YOUR_VULTR_API_KEY> --secret
pulumi config set allowedIngressIp <YOUR_HOME_IP>
pulumi config set rtmpConfig --secret "$(cat ../config.toml)"

pulumi up
```

When deployed, Pulumi encrypts `config.toml` into state and writes it securely to `/opt/rtmp-proxy/config.toml` (`chmod 600`) during instance startup.
