//! macOS banners for new activity. The bundled app posts through Notification
//! Center (asking permission once); a bare `cargo run` binary has no bundle,
//! which Notification Center requires, so it falls back to `osascript`.

use std::process::{Command, Stdio};

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError, NSString, NSUUID};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
    UNNotificationSound, UNUserNotificationCenter,
};

fn bundled() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

/// Asks for permission to show banners, once at launch; macOS remembers the answer.
pub fn request_permission() {
    if !bundled() {
        return;
    }
    let done = RcBlock::new(|granted: Bool, error: *mut NSError| {
        if !granted.as_bool() {
            // SAFETY: the error pointer is either null or a valid NSError for the call.
            let reason = unsafe { error.as_ref() }
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_else(|| "not allowed".to_string());
            eprintln!("meow: banners are off: {reason}");
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &done,
        );
}

/// Shows a banner: `title` in bold, `body` below.
pub fn post(title: &str, body: &str) {
    if bundled() {
        post_native(title, body);
    } else {
        post_script(title, body);
    }
}

fn post_native(title: &str, body: &str) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let id = NSUUID::new().UUIDString();
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
    let done = RcBlock::new(|error: *mut NSError| {
        // SAFETY: the error pointer is either null or a valid NSError for the call.
        if let Some(error) = unsafe { error.as_ref() } {
            eprintln!(
                "meow: could not show a banner: {}",
                error.localizedDescription()
            );
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&done));
}

fn post_script(title: &str, body: &str) {
    let script = format!(
        "display notification {} with title \"Meow\" subtitle {}",
        quoted(body),
        quoted(title)
    );
    let spawned = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(error) = spawned {
        eprintln!("meow: could not show a banner: {error}");
    }
}

/// An AppleScript string literal.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applescript_strings_are_escaped() {
        assert_eq!(quoted(r#"say "hi" \o/"#), r#""say \"hi\" \\o/""#);
    }
}
