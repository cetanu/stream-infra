# stream-infra

`stream-infra` is a self-hosted live-stream multiplexer. It accepts one RTMP
ingest and relays it to multiple configured destinations such as Twitch,
YouTube, and X.

The Rust service includes a server-rendered Topcoat dashboard for managing
stream targets, notifications, access credentials, and chat integrations
without handwritten JavaScript. Its unified chat inbox combines messages from
multiple platforms into a bounded queue, presenting one message at a time until
it is acknowledged.

Application configuration and chat state are persisted in SQLite. The
production infrastructure is managed with Pulumi on Vultr, where the database
lives on retained Block Storage independently of the replaceable compute
instance.

## Components

- Rust RTMP ingest, relay, health, and web services
- Topcoat-based configuration and chat dashboard
- Twitch EventSub and YouTube Live chat ingestion
- SQLite-backed configuration and chat queue
- Caddy TLS termination
- Pulumi-managed Vultr compute, networking, firewall, and Block Storage
