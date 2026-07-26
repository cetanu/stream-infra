use crate::chat::{EnqueueOutcome, IncomingChatMessage};
use crate::server::state::ProxyState;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::sync::Arc;
use topcoat::{
    context::{app_context, Cx},
    router::{route, Bytes, IntoResponse, StatusCode},
    Result,
};

const MESSAGE_ID_HEADER: &str = "twitch-eventsub-message-id";
const MESSAGE_TIMESTAMP_HEADER: &str = "twitch-eventsub-message-timestamp";
const MESSAGE_SIGNATURE_HEADER: &str = "twitch-eventsub-message-signature";
const MESSAGE_TYPE_HEADER: &str = "twitch-eventsub-message-type";

#[derive(Debug, Deserialize)]
struct TwitchEventSubPayload {
    #[serde(default)]
    challenge: Option<String>,
    #[serde(default)]
    event: Option<TwitchChatEvent>,
    #[serde(default)]
    subscription: Option<TwitchSubscription>,
}

#[derive(Debug, Deserialize)]
struct TwitchSubscription {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct TwitchChatEvent {
    message_id: String,
    chatter_user_name: String,
    message: TwitchChatMessage,
}

#[derive(Debug, Deserialize)]
struct TwitchChatMessage {
    text: String,
}

#[route(POST "/api/chat/twitch/eventsub")]
async fn twitch_eventsub(cx: &Cx, body: Bytes) -> Result<topcoat::router::Response> {
    let state: &Arc<ProxyState> = app_context(cx);
    let secret = state
        .config
        .read()
        .await
        .chat
        .twitch_eventsub_secret
        .clone()
        .filter(|value| !value.trim().is_empty());
    let Some(secret) = secret.as_deref() else {
        return IntoResponse::into_response(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Twitch EventSub ingest is disabled; configure its webhook secret",
            ),
            cx,
        );
    };

    let headers = topcoat::router::headers(cx);
    let message_id = header(headers, MESSAGE_ID_HEADER)?;
    let timestamp = header(headers, MESSAGE_TIMESTAMP_HEADER)?;
    let signature = header(headers, MESSAGE_SIGNATURE_HEADER)?;
    let message_type = header(headers, MESSAGE_TYPE_HEADER)?;

    if !timestamp_is_recent(timestamp)
        || !valid_signature(secret, message_id, timestamp, &body, signature)
    {
        return Err(topcoat::router::unauthorized().into());
    }

    let payload: TwitchEventSubPayload = serde_json::from_slice(&body).map_err(|error| {
        topcoat::router::bad_request(format!("Invalid Twitch payload: {error}"))
    })?;

    match message_type {
        "webhook_callback_verification" => {
            let challenge = payload.challenge.ok_or_else(|| {
                topcoat::router::bad_request("Twitch verification payload omitted its challenge")
            })?;
            IntoResponse::into_response(
                (
                    [(topcoat::router::header::CACHE_CONTROL, "no-store")],
                    challenge,
                ),
                cx,
            )
        }
        "notification" => {
            if payload
                .subscription
                .as_ref()
                .is_none_or(|subscription| subscription.kind != "channel.chat.message")
            {
                return Err(topcoat::router::bad_request(
                    "Unsupported Twitch EventSub subscription type",
                )
                .into());
            }
            let event = payload.event.ok_or_else(|| {
                topcoat::router::bad_request("Twitch notification omitted its event")
            })?;
            let incoming = IncomingChatMessage {
                source: "twitch".into(),
                external_id: event.message_id,
                author: event.chatter_user_name,
                text: event.message.text,
                avatar_url: None,
                sent_at: Some(timestamp.to_string()),
            };
            let mut inbox = state.chat_inbox.lock().await;
            match inbox.enqueue(incoming) {
                Ok(
                    EnqueueOutcome::Accepted | EnqueueOutcome::Duplicate | EnqueueOutcome::Dropped,
                ) => IntoResponse::into_response(StatusCode::NO_CONTENT, cx),
                Err(error) => Err(topcoat::router::bad_request(error.to_string()).into()),
            }
        }
        "revocation" => {
            tracing::warn!("Twitch revoked an EventSub chat subscription");
            IntoResponse::into_response(StatusCode::NO_CONTENT, cx)
        }
        _ => Err(topcoat::router::bad_request("Unsupported Twitch EventSub message type").into()),
    }
}

fn header<'a>(headers: &'a topcoat::router::HeaderMap, name: &'static str) -> Result<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| topcoat::router::bad_request(format!("Missing {name} header")).into())
}

fn valid_signature(
    secret: &str,
    message_id: &str,
    timestamp: &str,
    body: &[u8],
    submitted: &str,
) -> bool {
    let Some(submitted) = submitted.strip_prefix("sha256=").and_then(decode_sha256) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(message_id.as_bytes());
    mac.update(timestamp.as_bytes());
    mac.update(body);
    mac.verify_slice(&submitted).is_ok()
}

fn timestamp_is_recent(timestamp: &str) -> bool {
    let Ok(sent_at) =
        time::OffsetDateTime::parse(timestamp, &time::format_description::well_known::Rfc3339)
    else {
        return false;
    };
    let age = time::OffsetDateTime::now_utc() - sent_at;
    age >= -time::Duration::minutes(1) && age <= time::Duration::minutes(10)
}

fn decode_sha256(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }

    let mut bytes = [0_u8; 32];
    for (index, output) in bytes.iter_mut().enumerate() {
        let high = hex_digit(hex.as_bytes()[index * 2])?;
        let low = hex_digit(hex.as_bytes()[index * 2 + 1])?;
        *output = (high << 4) | low;
    }
    Some(bytes)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(secret: &str, message_id: &str, timestamp: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(message_id.as_bytes());
        mac.update(timestamp.as_bytes());
        mac.update(body);
        let bytes = mac.finalize().into_bytes();
        let hex = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        format!("sha256={hex}")
    }

    #[test]
    fn verifies_signed_twitch_payloads_and_rejects_tampering() {
        let body = br#"{"event":{"message_id":"one"}}"#;
        let submitted = signature("0123456789", "message-id", "timestamp", body);

        assert!(valid_signature(
            "0123456789",
            "message-id",
            "timestamp",
            body,
            &submitted
        ));
        assert!(!valid_signature(
            "0123456789",
            "message-id",
            "timestamp",
            b"tampered",
            &submitted
        ));
    }
}
