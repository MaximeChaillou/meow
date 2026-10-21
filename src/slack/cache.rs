//! What Meow last saw of Slack, kept on disk so the Slack page fills at once
//! on launch and people are not looked up again on every start.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::api::HISTORY_LIMIT;
use super::model::{Channel, Message, User};

/// People are looked up again after this long, in case their name changed.
pub const USER_TTL_SECS: u64 = 7 * 24 * 60 * 60;
/// Messages are kept for this many of the most recent conversations.
pub const MAX_CONVERSATIONS: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedUser {
    pub user: User,
    /// Unix seconds of the lookup.
    pub fetched_at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SlackCache {
    /// The Slack user id this belongs to; empty until known.
    pub owner: String,
    /// The conversation list, most recent first.
    pub channels: Vec<Channel>,
    pub users: HashMap<String, CachedUser>,
    /// The latest messages of recent conversations, oldest first.
    pub messages: HashMap<String, Vec<Message>>,
}

impl SlackCache {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("Could not read the Slack cache {}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Could not read {}: {e}", path.display())),
        }
    }

    /// Writes atomically, readable by the user only: it holds private messages.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let fail = |e: &dyn std::fmt::Display| {
            format!("Could not save the Slack cache to {}: {e}", path.display())
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
        }
        let json = serde_json::to_string(self).map_err(|e| fail(&e))?;
        let temp = path.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|e| fail(&e))?;
        file.write_all(json.as_bytes()).map_err(|e| fail(&e))?;
        std::fs::rename(&temp, path).map_err(|e| fail(&e))
    }

    /// This cache if it belongs to `owner` (or to no one yet), else an empty
    /// one for them: another account's messages are never shown.
    pub fn for_owner(self, owner: &str) -> Self {
        if self.owner.is_empty() || owner.is_empty() || self.owner == owner {
            let owner = if owner.is_empty() {
                self.owner
            } else {
                owner.to_string()
            };
            Self { owner, ..self }
        } else {
            Self {
                owner: owner.to_string(),
                ..Self::default()
            }
        }
    }

    /// Replaces the conversation list, and forgets the messages of
    /// conversations no longer among the most recent.
    pub fn set_channels(&mut self, channels: Vec<Channel>) {
        let recent: Vec<&str> = channels
            .iter()
            .take(MAX_CONVERSATIONS)
            .map(|c| c.id.as_str())
            .collect();
        self.messages.retain(|id, _| recent.contains(&id.as_str()));
        self.channels = channels;
    }

    pub fn add_users(&mut self, users: &[User], now: u64) {
        for user in users {
            let cached = CachedUser {
                user: user.clone(),
                fetched_at: now,
            };
            self.users.insert(user.id.clone(), cached);
        }
    }

    /// People looked up recently enough not to look up again.
    pub fn fresh_users(&self, now: u64) -> Vec<User> {
        self.users
            .values()
            .filter(|c| now.saturating_sub(c.fetched_at) < USER_TTL_SECS)
            .map(|c| c.user.clone())
            .collect()
    }

    /// Names of every cached person, for titles and `mrkdwn`.
    pub fn names(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.users
            .iter()
            .map(|(id, c)| (id.clone(), c.user.name().to_string()))
    }

    pub fn history(&self, channel: &str) -> &[Message] {
        self.messages.get(channel).map_or(&[], Vec::as_slice)
    }

    /// Stores a conversation's latest messages, as just loaded.
    pub fn set_history(&mut self, channel: &str, messages: &[Message]) {
        self.messages
            .insert(channel.to_string(), latest(messages.to_vec()));
    }

    /// Adds messages that just arrived to a conversation's stored ones.
    pub fn append(&mut self, channel: &str, messages: &[Message]) {
        let stored = self.messages.entry(channel.to_string()).or_default();
        let mut all = std::mem::take(stored);
        all.extend(
            messages
                .iter()
                .filter(|m| !all.iter().any(|s| s.ts == m.ts))
                .cloned()
                .collect::<Vec<_>>(),
        );
        all.sort_by(|a, b| a.ts.cmp(&b.ts));
        *stored = latest(all);
    }
}

/// The last `HISTORY_LIMIT` messages.
fn latest(mut messages: Vec<Message>) -> Vec<Message> {
    let excess = messages.len().saturating_sub(HISTORY_LIMIT as usize);
    messages.drain(..excess);
    messages
}

/// `~/Library/Application Support/Meow/slack-cache.json`.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/Meow/slack-cache.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slack::model::ChannelKind;

    fn channel(id: &str) -> Channel {
        Channel {
            id: id.into(),
            kind: ChannelKind::Im,
            name: String::new(),
            user: Some(format!("U{id}")),
            latest_ts: None,
            unread: false,
            mentions: 0,
        }
    }

    fn message(ts: &str) -> Message {
        Message {
            ts: ts.into(),
            user: Some("U1".into()),
            text: format!("at {ts}"),
            thread_ts: None,
            reply_count: 0,
            files: Vec::new(),
            images: Vec::new(),
            attachment: None,
            bot_name: None,
            forwarded: Vec::new(),
        }
    }

    fn user(id: &str, name: &str) -> User {
        User {
            id: id.into(),
            real_name: name.into(),
            display_name: String::new(),
            email: String::new(),
            tz: String::new(),
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("meow-slack-cache-{}", std::process::id()));
        let path = dir.join("slack-cache.json");
        let mut cache = SlackCache {
            owner: "U1".into(),
            ..SlackCache::default()
        };
        cache.set_channels(vec![channel("D1")]);
        cache.add_users(&[user("U2", "Bob")], 100);
        cache.set_history("D1", &[message("1.0")]);
        cache.save(&path).unwrap();
        assert_eq!(SlackCache::load(&path).unwrap(), cache);
        let mode = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
            0o600
        );
        assert_eq!(
            SlackCache::load(&dir.join("missing.json")).unwrap(),
            SlackCache::default()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn another_account_starts_afresh() {
        let mut cache = SlackCache {
            owner: "U1".into(),
            ..SlackCache::default()
        };
        cache.set_channels(vec![channel("D1")]);
        assert_eq!(cache.clone().for_owner("U1").channels.len(), 1);
        assert_eq!(cache.clone().for_owner("").channels.len(), 1);
        let other = cache.for_owner("U9");
        assert_eq!(other.owner, "U9");
        assert!(other.channels.is_empty());
        let unowned = SlackCache::default().for_owner("U3");
        assert_eq!(unowned.owner, "U3");
    }

    #[test]
    fn people_are_looked_up_again_after_a_week() {
        let mut cache = SlackCache::default();
        cache.add_users(&[user("U2", "Bob")], 1_000);
        assert_eq!(cache.fresh_users(1_000 + 60).len(), 1);
        assert!(cache.fresh_users(1_000 + USER_TTL_SECS).is_empty());
        assert_eq!(
            cache.names().collect::<Vec<_>>(),
            vec![("U2".to_string(), "Bob".to_string())]
        );
    }

    #[test]
    fn history_keeps_the_latest_messages_once_each() {
        let mut cache = SlackCache::default();
        let many: Vec<Message> = (0..60).map(|i| message(&format!("{:04}.0", i))).collect();
        cache.set_history("D1", &many);
        assert_eq!(cache.history("D1").len(), HISTORY_LIMIT as usize);
        assert_eq!(cache.history("D1").last().unwrap().ts, "0059.0");

        cache.append("D1", &[message("0059.0"), message("0060.0")]);
        let stored = cache.history("D1");
        assert_eq!(stored.len(), HISTORY_LIMIT as usize);
        assert_eq!(stored.last().unwrap().ts, "0060.0");
        assert_eq!(stored.iter().filter(|m| m.ts == "0059.0").count(), 1);
        assert!(cache.history("D9").is_empty());
    }

    #[test]
    fn messages_of_dropped_conversations_are_forgotten() {
        let mut cache = SlackCache::default();
        cache.set_history("D1", &[message("1.0")]);
        cache.set_history("D2", &[message("1.0")]);
        let mut channels = vec![channel("D2")];
        channels.extend((0..MAX_CONVERSATIONS).map(|i| channel(&format!("C{i}"))));
        cache.set_channels(channels);
        assert!(cache.history("D1").is_empty(), "no longer listed");
        assert_eq!(cache.history("D2").len(), 1);

        let mut channels: Vec<Channel> = (0..MAX_CONVERSATIONS)
            .map(|i| channel(&format!("C{i}")))
            .collect();
        channels.push(channel("D2"));
        cache.set_channels(channels);
        assert!(cache.history("D2").is_empty(), "too far down the list");
    }
}
