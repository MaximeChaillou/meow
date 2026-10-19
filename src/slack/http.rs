use std::time::Duration;

use ureq::Agent;

/// What a request came back with. A non-2xx `status` still carries whatever body
/// Slack sent, since the Web API reports its own errors with HTTP 200.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

/// A form POST to a Slack method, with the session cookie.
pub trait Http {
    /// POSTs `params` (url-encoded) to `url` with `Cookie: d=<cookie>`.
    fn post_form(
        &self,
        url: &str,
        cookie: &str,
        params: &[(&str, &str)],
    ) -> Result<Response, String>;
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TIME: Duration = Duration::from_secs(30);

/// The production transport.
#[derive(Clone, Debug)]
pub struct Web {
    agent: Agent,
}

impl Web {
    pub fn new() -> Self {
        let config = Agent::config_builder()
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(MAX_TIME))
            // Hand non-2xx responses back with their body (e.g. 429 rate limits).
            .http_status_as_error(false)
            .build();
        Self {
            agent: Agent::new_with_config(config),
        }
    }
}

impl Default for Web {
    fn default() -> Self {
        Self::new()
    }
}

impl Http for Web {
    fn post_form(
        &self,
        url: &str,
        cookie: &str,
        params: &[(&str, &str)],
    ) -> Result<Response, String> {
        let response = self
            .agent
            .post(url)
            .header("Cookie", format!("d={cookie}"))
            .send_form(params.iter().copied())
            .map_err(|e| format!("request failed: {e}"))?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_string()
            .map_err(|e| format!("could not read response: {e}"))?;
        Ok(Response { status, body })
    }
}
