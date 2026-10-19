//! Stores the Slack credentials in the macOS Keychain.
//!
//! The token and cookie are as powerful as the login itself, so they live only
//! here, never in a file or a log. They are kept as one JSON blob under a single
//! generic-password item.

use security_framework::base::Error as KeychainError;
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

use super::auth::Credentials;

const SERVICE: &str = "Meow";
const ACCOUNT: &str = "slack-credentials";
/// `errSecItemNotFound`: nothing stored yet.
const NOT_FOUND: i32 = -25300;

/// Saves the credentials, replacing any already stored.
pub fn save(creds: &Credentials) -> Result<(), String> {
    let json = serde_json::to_vec(creds).map_err(|e| e.to_string())?;
    set_generic_password(SERVICE, ACCOUNT, &json)
        .map_err(|e| format!("could not save Slack sign-in to the Keychain: {e}"))
}

/// Loads the stored credentials, or `None` when the user has not signed in.
pub fn load() -> Result<Option<Credentials>, String> {
    match get_generic_password(SERVICE, ACCOUNT) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("stored Slack sign-in was unreadable: {e}")),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(format!("could not read the Keychain: {e}")),
    }
}

/// Removes the stored credentials, e.g. after Slack ends the session.
pub fn clear() -> Result<(), String> {
    match delete_generic_password(SERVICE, ACCOUNT) {
        Ok(()) => Ok(()),
        Err(e) if is_not_found(&e) => Ok(()),
        Err(e) => Err(format!("could not clear the Keychain: {e}")),
    }
}

fn is_not_found(error: &KeychainError) -> bool {
    error.code() == NOT_FOUND
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Credentials {
        Credentials {
            token: "xoxc-test".into(),
            cookie: "xoxd-test".into(),
            team_id: "T1".into(),
            domain: "acme".into(),
            user_id: "U1".into(),
        }
    }

    // Touches the real login Keychain (and may prompt), so it is opt-in:
    //   cargo test --lib slack::keychain -- --ignored
    #[test]
    #[ignore]
    fn round_trips_through_the_keychain() {
        save(&sample()).unwrap();
        assert_eq!(load().unwrap(), Some(sample()));
        clear().unwrap();
        assert_eq!(load().unwrap(), None);
    }
}
