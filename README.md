# stream-infra

Application-agnostic Pulumi infrastructure for a small Vultr compute node.

Pulumi provisions the stable host, firewall, Caddy, and a signed GitHub webhook
listener. Webhooks activate a masterless Salt reconciliation through systemd.
Salt loads each application's formula directly from its own Git repository via
GitFS, allowing applications to be replaced and restarted without updating or
replacing the compute node.

The RTMP application now lives at
[`cetanu/rtmp-manager`](https://github.com/cetanu/rtmp-manager).

See [`infra/README.md`](infra/README.md) for configuration and deployment.
