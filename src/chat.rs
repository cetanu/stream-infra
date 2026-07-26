use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod youtube;

pub const DEFAULT_CHAT_QUEUE_CAPACITY: usize = 500;

#[derive(Debug, Clone, Deserialize)]
pub struct IncomingChatMessage {
    pub source: String,
    pub external_id: String,
    pub author: String,
    pub text: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub sent_at: Option<String>,
}

impl IncomingChatMessage {
    fn normalized(mut self) -> Result<Self> {
        self.source = self.source.trim().to_ascii_lowercase();
        self.external_id = self.external_id.trim().to_string();
        self.author = self.author.trim().to_string();
        self.text = self.text.trim().to_string();
        self.avatar_url = non_empty(self.avatar_url);
        self.sent_at = non_empty(self.sent_at);

        if self.source.is_empty()
            || self.source.len() > 32
            || !self.source.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            bail!("source must be 1-32 ASCII letters, numbers, hyphens, or underscores");
        }
        if self.external_id.is_empty() || self.external_id.len() > 256 {
            bail!("external_id must be 1-256 characters");
        }
        if self.author.is_empty() || self.author.len() > 200 {
            bail!("author must be 1-200 characters");
        }
        if self.text.is_empty() || self.text.len() > 5000 {
            bail!("text must be 1-5000 characters");
        }
        if self.avatar_url.as_ref().is_some_and(|url| url.len() > 2048) {
            bail!("avatar_url must be at most 2048 characters");
        }
        if self
            .sent_at
            .as_ref()
            .is_some_and(|timestamp| timestamp.len() > 100)
        {
            bail!("sent_at must be at most 100 characters");
        }

        Ok(self)
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub id: u64,
    pub source: String,
    pub external_id: String,
    pub author: String,
    pub text: String,
    pub avatar_url: Option<String>,
    pub sent_at: Option<String>,
    pub received_at_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatInboxSnapshot {
    pub current: Option<ChatMessage>,
    pub waiting: usize,
    pub dropped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Accepted,
    Duplicate,
    Dropped,
}

#[derive(Debug)]
pub struct ChatInbox {
    capacity: usize,
    next_id: u64,
    messages: VecDeque<ChatMessage>,
    seen: HashSet<String>,
    seen_order: VecDeque<String>,
    dropped: u64,
}

impl ChatInbox {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "chat queue capacity must be positive");
        Self {
            capacity,
            next_id: 1,
            messages: VecDeque::with_capacity(capacity),
            seen: HashSet::with_capacity(capacity * 2),
            seen_order: VecDeque::with_capacity(capacity * 2),
            dropped: 0,
        }
    }

    pub fn enqueue(&mut self, incoming: IncomingChatMessage) -> Result<EnqueueOutcome> {
        let incoming = incoming.normalized()?;
        let deduplication_key = format!("{}:{}", incoming.source, incoming.external_id);
        if self.seen.contains(&deduplication_key) {
            return Ok(EnqueueOutcome::Duplicate);
        }

        self.remember(deduplication_key);

        if self.messages.len() == self.capacity {
            self.dropped += 1;
            if self.capacity == 1 {
                return Ok(EnqueueOutcome::Dropped);
            }
            self.messages.remove(1);
        }

        let message = ChatMessage {
            id: self.next_id,
            source: incoming.source,
            external_id: incoming.external_id,
            author: incoming.author,
            text: incoming.text,
            avatar_url: incoming.avatar_url,
            sent_at: incoming.sent_at,
            received_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };
        self.next_id = self.next_id.saturating_add(1);
        self.messages.push_back(message);
        Ok(EnqueueOutcome::Accepted)
    }

    pub fn acknowledge(&mut self, expected_id: u64) -> bool {
        if self
            .messages
            .front()
            .is_some_and(|message| message.id == expected_id)
        {
            self.messages.pop_front();
            true
        } else {
            false
        }
    }

    pub fn snapshot(&self) -> ChatInboxSnapshot {
        ChatInboxSnapshot {
            current: self.messages.front().cloned(),
            waiting: self.messages.len().saturating_sub(1),
            dropped: self.dropped,
        }
    }

    fn remember(&mut self, key: String) {
        let seen_capacity = self.capacity.saturating_mul(4);
        self.seen.insert(key.clone());
        self.seen_order.push_back(key);

        while self.seen_order.len() > seen_capacity {
            if let Some(expired) = self.seen_order.pop_front() {
                self.seen.remove(&expired);
            }
        }
    }
}

impl Default for ChatInbox {
    fn default() -> Self {
        Self::new(DEFAULT_CHAT_QUEUE_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(source: &str, external_id: &str, text: &str) -> IncomingChatMessage {
        IncomingChatMessage {
            source: source.into(),
            external_id: external_id.into(),
            author: "Viewer".into(),
            text: text.into(),
            avatar_url: None,
            sent_at: None,
        }
    }

    #[test]
    fn acknowledge_advances_one_message_at_a_time() {
        let mut inbox = ChatInbox::new(3);
        inbox.enqueue(message("twitch", "1", "first")).unwrap();
        inbox.enqueue(message("youtube", "2", "second")).unwrap();

        let first = inbox.snapshot();
        assert_eq!(first.current.unwrap().text, "first");
        assert_eq!(first.waiting, 1);
        assert!(inbox.acknowledge(1));

        let second = inbox.snapshot();
        assert_eq!(second.current.unwrap().text, "second");
        assert_eq!(second.waiting, 0);
    }

    #[test]
    fn stale_acknowledgement_cannot_clear_a_newer_message() {
        let mut inbox = ChatInbox::new(3);
        inbox.enqueue(message("twitch", "1", "first")).unwrap();
        inbox.enqueue(message("twitch", "2", "second")).unwrap();
        assert!(inbox.acknowledge(1));
        assert!(!inbox.acknowledge(1));
        assert_eq!(inbox.snapshot().current.unwrap().text, "second");
    }

    #[test]
    fn duplicate_platform_message_is_ignored() {
        let mut inbox = ChatInbox::new(3);
        assert_eq!(
            inbox.enqueue(message("twitch", "same", "first")).unwrap(),
            EnqueueOutcome::Accepted
        );
        assert_eq!(
            inbox
                .enqueue(message("twitch", "same", "duplicate"))
                .unwrap(),
            EnqueueOutcome::Duplicate
        );
        assert_eq!(inbox.snapshot().waiting, 0);
    }

    #[test]
    fn full_queue_keeps_current_and_most_recent_waiting_messages() {
        let mut inbox = ChatInbox::new(3);
        inbox.enqueue(message("twitch", "1", "current")).unwrap();
        inbox
            .enqueue(message("twitch", "2", "old waiting"))
            .unwrap();
        inbox
            .enqueue(message("youtube", "3", "newer waiting"))
            .unwrap();
        inbox.enqueue(message("x", "4", "newest waiting")).unwrap();

        assert_eq!(inbox.snapshot().current.unwrap().text, "current");
        assert_eq!(inbox.snapshot().waiting, 2);
        assert_eq!(inbox.snapshot().dropped, 1);
        assert!(inbox.acknowledge(1));
        assert_eq!(inbox.snapshot().current.unwrap().text, "newer waiting");
    }
}
