//! The credentials Meow reads once at sign-in, and how to pull them out of the
//! Slack web app's own storage.
//!
//! The web client keeps a session token (`xoxc-…`) per workspace in its
//! `localConfig_v2` local-storage entry, and sends the `d` cookie (`xoxd-…`)
//! alongside it. Both are needed; either alone is rejected. This module parses
//! the local-storage blob (pure, tested here); reading the cookie and storing
//! the pair in the Keychain live with the macOS-facing code.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Everything needed to talk to Slack as the signed-in user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    /// The session token, `xoxc-…`.
    pub token: String,
    /// The `d` cookie value, `xoxd-…`.
    pub cookie: String,
    pub team_id: String,
    /// The workspace subdomain, e.g. `acme` in `acme.slack.com`.
    pub domain: String,
    pub user_id: String,
}

impl Credentials {
    /// Whether the two secrets at least look like Slack's own.
    pub fn looks_valid(&self) -> bool {
        self.token.starts_with("xoxc-") && self.cookie.starts_with("xoxd-")
    }
}

/// One workspace found in `localConfig_v2`, before the `d` cookie is attached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub team_id: String,
    pub token: String,
    pub domain: String,
    pub name: String,
    pub user_id: String,
}

impl Workspace {
    /// Completes the credentials with the `d` cookie read from the cookie store.
    pub fn with_cookie(self, cookie: impl Into<String>) -> Credentials {
        Credentials {
            token: self.token,
            cookie: cookie.into(),
            team_id: self.team_id,
            domain: self.domain,
            user_id: self.user_id,
        }
    }
}

/// Parses the `localConfig_v2` local-storage value into its workspaces.
pub fn parse_local_config(json: &str) -> Result<Vec<Workspace>, String> {
    let config: LocalConfig =
        serde_json::from_str(json).map_err(|e| format!("could not read Slack config: {e}"))?;
    let mut workspaces: Vec<Workspace> = config
        .teams
        .into_iter()
        .filter(|(_, team)| !team.token.is_empty())
        .map(|(id, team)| Workspace {
            team_id: if team.id.is_empty() { id } else { team.id },
            token: team.token,
            domain: team.domain,
            name: team.name,
            user_id: team.user_id,
        })
        .collect();
    // Stable order so selection and tests are deterministic.
    workspaces.sort_by(|a, b| a.team_id.cmp(&b.team_id));
    if workspaces.is_empty() {
        return Err("no signed-in Slack workspace was found".into());
    }
    Ok(workspaces)
}

/// Chooses the workspace to use. A `hint` matching a team id or subdomain (the
/// one the user is signed in on) wins; otherwise the single workspace, or `None`
/// when it is ambiguous.
pub fn select_workspace(workspaces: &[Workspace], hint: Option<&str>) -> Option<Workspace> {
    if let Some(hint) = hint
        && let Some(matched) = workspaces
            .iter()
            .find(|w| w.team_id == hint || w.domain == hint)
    {
        return Some(matched.clone());
    }
    match workspaces {
        [only] => Some(only.clone()),
        _ => None,
    }
}

// --- Wire format ------------------------------------------------------------

#[derive(Deserialize)]
struct LocalConfig {
    #[serde(default)]
    teams: HashMap<String, RawTeam>,
}

#[derive(Deserialize)]
struct RawTeam {
    #[serde(default)]
    token: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    domain: String,
    #[serde(default)]
    user_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"{
        "teams": {
            "T1": {"id":"T1","token":"xoxc-aaa","name":"Acme","domain":"acme","user_id":"U1"},
            "T2": {"id":"T2","token":"xoxc-bbb","name":"Side","domain":"side","user_id":"U2"}
        },
        "lastActiveTeamId": "T2"
    }"#;

    #[test]
    fn parses_each_workspace() {
        let workspaces = parse_local_config(CONFIG).unwrap();
        assert_eq!(workspaces.len(), 2);
        assert_eq!(workspaces[0].team_id, "T1");
        assert_eq!(workspaces[0].token, "xoxc-aaa");
        assert_eq!(workspaces[0].domain, "acme");
    }

    #[test]
    fn skips_teams_without_a_token() {
        let json = r#"{"teams":{"T1":{"id":"T1","domain":"a"},"T2":{"id":"T2","token":"xoxc-x","domain":"b"}}}"#;
        let workspaces = parse_local_config(json).unwrap();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].domain, "b");
    }

    #[test]
    fn empty_config_is_an_error() {
        assert!(parse_local_config(r#"{"teams":{}}"#).is_err());
        assert!(parse_local_config("garbage").is_err());
    }

    #[test]
    fn domain_hint_selects_the_matching_workspace() {
        let workspaces = parse_local_config(CONFIG).unwrap();
        let chosen = select_workspace(&workspaces, Some("side")).unwrap();
        assert_eq!(chosen.team_id, "T2");
    }

    #[test]
    fn team_id_hint_selects_the_matching_workspace() {
        let workspaces = parse_local_config(CONFIG).unwrap();
        let chosen = select_workspace(&workspaces, Some("T1")).unwrap();
        assert_eq!(chosen.domain, "acme");
    }

    #[test]
    fn single_workspace_is_chosen_without_a_hint() {
        let one = vec![Workspace {
            team_id: "T1".into(),
            token: "xoxc-aaa".into(),
            domain: "acme".into(),
            name: "P".into(),
            user_id: "U1".into(),
        }];
        assert!(select_workspace(&one, None).is_some());
    }

    #[test]
    fn ambiguous_without_a_hint_is_none() {
        let workspaces = parse_local_config(CONFIG).unwrap();
        assert!(select_workspace(&workspaces, None).is_none());
        assert!(select_workspace(&workspaces, Some("unknown")).is_none());
    }

    #[test]
    fn cookie_completes_the_credentials() {
        let workspace = Workspace {
            team_id: "T1".into(),
            token: "xoxc-aaa".into(),
            domain: "acme".into(),
            name: "P".into(),
            user_id: "U1".into(),
        };
        let creds = workspace.with_cookie("xoxd-zzz");
        assert!(creds.looks_valid());
        assert_eq!(creds.token, "xoxc-aaa");
        assert_eq!(creds.cookie, "xoxd-zzz");
        assert_eq!(creds.user_id, "U1");
    }

    #[test]
    fn credentials_reject_wrong_prefixes() {
        let bad = Credentials {
            token: "nope".into(),
            cookie: "xoxd-z".into(),
            team_id: "T1".into(),
            domain: "d".into(),
            user_id: "U1".into(),
        };
        assert!(!bad.looks_valid());
    }
}
