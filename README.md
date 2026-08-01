# Application host infrastructure

This Pulumi project provisions a stable Vultr compute node and a generic,
event-driven application reconciler. Applications are managed independently by
masterless Salt states; changing an application does not replace the compute
instance.

## Deployment flow

1. GitHub sends a `release.published` webhook over HTTPS.
2. Caddy forwards the request to an unprivileged listener on localhost.
3. The listener verifies `X-Hub-Signature-256`, the event, action, and numeric
   repository ID.
4. It durably creates `/var/lib/deployment-reconciler/pending` and returns 202.
5. A systemd path unit starts `deployment-reconcile.service`.
6. Salt refreshes the configured GitFS remotes and applies the local top file.
7. Salt downloads changed artifacts and restarts only affected services.

There is intentionally no timer, cron job, or periodic Git polling. An accepted
webhook survives a restart through the durable marker. If GitHub cannot deliver
a webhook, redeliver it explicitly from GitHub.

## Required stack configuration

From the repository root:

```sh
pulumi config set vultr:apiKey --secret
pulumi config set stateRepositories "$(jq -c . state-repositories.example.json)"
pulumi config set webhookHost deploy.example.com
pulumi config set webhookSecret --secret
pulumi config set ddnsRecords '[{"host":"rtmp","domain":"example.com"},{"host":"deploy","domain":"example.com"}]'
pulumi config set ddnsPassword --secret
pulumi config set gpgPrivateKey --secret < gpg-private-key.asc
pulumi config set firewallRules "$(jq -c . firewall-rules.example.json)"
```

Each state repository must currently be readable without interactive
authentication. Salt uses GitPython for both its masterless GitFS state backend
and Git external pillar. Each repository owns encrypted pillar data; Pulumi
holds only the host's armored GPG private key and installs it into the root-only
`/etc/salt/gpgkeys` keyring.

Each `stateRepositories` entry contains:

- `url`: Git remote containing the formula
- `branch`: branch exposed as Salt's `base` environment
- `root`: repository-relative formula root
- `pillarRoot`: repository-relative encrypted pillar root
- `state`: state included by the host's local `top.sls`
- `repositoryId`: immutable GitHub repository ID allowed to trigger deployment

Salt requires successful GPG decryption. A missing key or malformed ciphertext
fails the reconciliation instead of writing ciphertext into an application
secret file.

## GPG setup

Generate one unprotected deployment key offline, back up the private key, and
export both forms:

```sh
gpg --quick-generate-key 'stream-infra deployment' rsa4096 encr never
gpg --armor --export 'stream-infra deployment' > salt-public-key.asc
gpg --armor --export-secret-keys 'stream-infra deployment' > gpg-private-key.asc
```

Configure `gpg-private-key.asc` through the secret Pulumi input and commit only
`salt-public-key.asc` to application repositories. Encrypt individual values:

```sh
printf %s 'secret value' | gpg --armor --encrypt --recipient 'stream-infra deployment'
```

Application pillar files use `#!yaml|gpg`; Salt decrypts their armored values
during pillar compilation. Secret-writing states should use mode `0600` and
`show_changes: false`.

Optional webhook settings are `webhookPath` (default `/hooks/github`),
`webhookEvent` (default `release`), `webhookAction` (default `published`), and
`webhookListenPort` (default `9100`, bound only to localhost).

Optional compute settings are `resourcePrefix`, `description`, `region`,
`plan`, `osId`, `label`, `hostname`, `vpcSubnet`, `vpcSubnetMask`,
`enableIPv6`, and `backups`.

## GitHub webhook

Create a repository webhook with:

- Payload URL: `https://deploy.example.com/hooks/github`
- Content type: `application/json`
- Secret: exactly the value configured as `webhookSecret`
- Events: Releases

DNS for `webhookHost` must point at the instance so Caddy can obtain its TLS
certificate. The host updates every Namecheap Dynamic DNS record in
`ddnsRecords` at boot and every five minutes using the secret `ddnsPassword`.
The list must include the fully-qualified `webhookHost`; it can also include
application records such as `rtmp.example.com`. The Vultr firewall must admit
public TCP 80 and 443 for ACME and the webhook. The internal listener port must
not be exposed.

## Adding applications

Add another object to `stateRepositories`. Pulumi generates the local top file,
allows signed release webhooks from that repository ID, mounts its formula
through GitFS, and loads its encrypted Git pillar. Each application repository
owns its artifact, secrets, systemd unit, health checks, and restart requisites.

The first example links
[`cetanu/rtmp-manager`](https://github.com/cetanu/rtmp-manager), whose formula
lives under its `salt` directory.

Before updating an existing stack, configure the required values above and run
`pulumi preview`. The new host bootstrap and changed logical resource names can
require replacement during this one-time migration; after that, application
releases do not involve Pulumi.
