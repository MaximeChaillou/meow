//! Live Slack chat: sign-in, Web API, message model and the live connection.
//!
//! Meow talks to Slack the way the web client does, with a session token
//! (`xoxc-…`) and the `d` cookie (`xoxd-…`) read once at sign-in. No Slack app
//! is registered. Networking runs on a background thread and reaches the 30 Hz
//! tick over an `mpsc` channel, like [`crate::claude`].

pub mod api;
pub mod assist;
pub mod auth;
pub mod cache;
pub mod calendar;
pub mod client;
pub mod format;
pub mod http;
pub mod keychain;
pub mod model;
pub mod timeline;
pub mod voice;
