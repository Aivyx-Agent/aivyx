//! Atlas (AT.3) recommended every tool-process crate run
//! `check_tool_quality` over the tools it owns. This is
//! `aivyx-calendar`'s sweep: each tool's name / description /
//! schema — the metadata the LLM relies on to call it — must
//! meet the correctness floor.

use std::path::PathBuf;
use std::sync::Arc;

use aivyx_calendar::oauth::{OAuthConfig, TokenSet};
use aivyx_calendar::tools::{
    CalendarCreateEvent, CalendarDeleteEvent, CalendarGetEvent, CalendarListCalendars,
    CalendarListEvents, CalendarUpcoming, CalendarUpdateEvent,
};
use aivyx_calendar::CalendarClient;
use aivyx_core::tools::check_tool_quality;
use aivyx_core::Tool;

fn fake_client() -> Arc<CalendarClient> {
    let config = OAuthConfig::new(
        "test-client-id",
        "test-client-secret",
        "http://127.0.0.1:0/callback",
    )
    .with_scopes(["https://www.googleapis.com/auth/calendar"]);
    let tokens = TokenSet {
        access_token: "fake-access-token".to_string(),
        refresh_token: Some("fake-refresh-token".to_string()),
        expires_at_unix_secs: 0,
        granted_scope: "https://www.googleapis.com/auth/calendar".to_string(),
        token_type: "Bearer".to_string(),
    };
    Arc::new(CalendarClient::new(
        reqwest::Client::new(),
        config,
        tokens,
        PathBuf::from("/tmp/aivyx-calendar-tool-quality-test-tokens.json"),
    ))
}

#[test]
fn calendar_tools_meet_quality_floor() {
    let client = fake_client();
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(CalendarListEvents::new(client.clone())),
        Arc::new(CalendarGetEvent::new(client.clone())),
        Arc::new(CalendarCreateEvent::new(client.clone())),
        Arc::new(CalendarUpdateEvent::new(client.clone())),
        Arc::new(CalendarDeleteEvent::new(client.clone())),
        Arc::new(CalendarListCalendars::new(client.clone())),
        Arc::new(CalendarUpcoming::new(client)),
    ];

    let issues: Vec<String> = tools
        .iter()
        .flat_map(|t| check_tool_quality(t.as_ref()))
        .collect();

    assert!(issues.is_empty(), "calendar tool quality issues:\n  {}", issues.join("\n  "));
}
