//! The POST to the telemetry server's `/v1/report` and its answer (docs/telemetry.md): 201 with
//! the issue's link, 202 when the report is taken and its issue opens in a moment, an error with
//! `{"error": "..."}` otherwise (a limit reached, a field refused, reports closed).
use serde_json::Value;
use std::time::Duration;

/// What the server did with the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sent {
    /// The issue's link.
    Opened(String),
    /// Kept; its issue opens soon, and the same report sent again gets it.
    Queued,
}

/// Posts `body` to `endpoint` (`https://host`, no trailing slash). Blocks: call it from a thread.
pub fn post(endpoint: &str, body: &Value) -> Result<Sent, String> {
    let agent = ureq::AgentBuilder::new()
        .user_agent("Horizon")
        .timeout(Duration::from_secs(30))
        .build();
    let result = agent
        .post(&format!("{endpoint}/v1/report"))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string());
    match result {
        Ok(response) => {
            let status = response.status();
            let text = response.into_string().map_err(|e| e.to_string())?;
            answer(status, &text)
        }
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            answer(status, &text)
        }
        Err(e) => Err(e.to_string()),
    }
}

/// The server's answer to the report as a result.
pub fn answer(status: u16, text: &str) -> Result<Sent, String> {
    let json: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    match status {
        201 | 202 => match json["issue"].as_str() {
            Some(link) if link.starts_with("https://") => Ok(Sent::Opened(link.to_owned())),
            _ => Ok(Sent::Queued),
        },
        _ => {
            let why = json["error"].as_str().unwrap_or("no reason given");
            Err(format!("{status}: {}", crate::body::shorten(why, 200)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opened_issue_gives_its_link() {
        let text = r#"{"id":7,"issue":"https://github.com/horizon-rust/horizon/issues/1"}"#;
        assert_eq!(
            answer(201, text),
            Ok(Sent::Opened(
                "https://github.com/horizon-rust/horizon/issues/1".into()
            ))
        );
    }

    #[test]
    fn a_queued_report_has_no_link_yet() {
        assert_eq!(answer(202, r#"{"id":7,"issue":null}"#), Ok(Sent::Queued));
    }

    #[test]
    fn a_refusal_gives_the_servers_reason() {
        assert_eq!(
            answer(400, r#"{"error":"happened: 10 to 500 characters"}"#),
            Err("400: happened: 10 to 500 characters".to_owned())
        );
        assert_eq!(
            answer(502, "<html>"),
            Err("502: no reason given".to_owned())
        );
    }
}
