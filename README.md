# stream-infra

RTMP Stream Multiplexer powered by `rtmp-rs` and provisioned on Vultr with Pulumi

## Unified chat inbox

The web dashboard includes a bounded, one-message-at-a-time chat inbox. Messages
from different platforms enter the same FIFO queue. The currently displayed
message remains visible until it is acknowledged, then the next waiting message
is shown.

The inbox holds up to 500 messages in memory by default. Set
`CHAT_QUEUE_CAPACITY` to a positive integer to change the bound. If it fills,
the current message is preserved and the oldest waiting message is discarded.
Platform retries are deduplicated by the combination of `source` and
`external_id`.

### Sending messages to the inbox

Set a strong bearer token before starting the service:

```sh
export CHAT_INGEST_TOKEN="replace-with-a-long-random-value"
cargo run
```

For systemd installations, put the token in
`/opt/rtmp-proxy/rtmp-proxy.env` (or the configured work directory):

```text
CHAT_INGEST_TOKEN=replace-with-a-long-random-value
```

An ingest adapter sends each platform message to the normalized endpoint:

```sh
curl --request POST http://127.0.0.1:3000/api/chat/ingest \
  --header "Authorization: Bearer $CHAT_INGEST_TOKEN" \
  --header "Content-Type: application/json" \
  --data '{
    "source": "twitch",
    "external_id": "platform-message-id",
    "author": "viewer-name",
    "text": "Hello from chat",
    "avatar_url": "https://example.com/avatar.png",
    "sent_at": "2026-07-26T07:30:00Z"
  }'
```

`source` is intentionally platform-neutral and can be `twitch`, `youtube`, `x`,
or another short adapter name. `avatar_url` and `sent_at` are optional.

The endpoint is disabled when `CHAT_INGEST_TOKEN` is unset. Direct Twitch and
YouTube adapters can also write directly to the same inbox.

### Twitch EventSub

Set a Twitch webhook secret of 10-100 characters:

```text
TWITCH_EVENTSUB_SECRET=replace-with-the-eventsub-webhook-secret
```

Register a Twitch
[`channel.chat.message`](https://dev.twitch.tv/docs/eventsub/eventsub-subscription-types/#channelchatmessage)
EventSub webhook with this callback:

```text
https://your-stream-host.example/api/chat/twitch/eventsub
```

The callback verifies every Twitch HMAC signature before accepting a message
and supports Twitch's callback challenge and revocation messages. Twitch
requires an HTTPS callback on port 443 and the appropriate broadcaster/bot
authorization when the EventSub subscription is created.

### YouTube Live

Configure both values to start the built-in YouTube polling collector:

```text
YOUTUBE_API_KEY=replace-with-a-google-api-key
YOUTUBE_LIVE_CHAT_ID=replace-with-the-active-live-chat-id
```

The collector follows YouTube's `nextPageToken` and
`pollingIntervalMillis`, includes displayable chat and paid-message events, and
reconnects with bounded exponential backoff. The active `liveChatId` is exposed
by the broadcast's `snippet.liveChatId` field. See the official
[`liveChatMessages.list`](https://developers.google.com/youtube/v3/live/docs/liveChatMessages/list)
documentation.

### X Live

X does not currently expose an official X Live video-chat API. An X integration
therefore needs a separate bridge that emits the normalized payload above with
`"source": "x"`. This project intentionally does not scrape X's private web
endpoints.
