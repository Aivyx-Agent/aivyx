//! Atlas (AT.3) recommended every tool-process crate run
//! `check_tool_quality` over the tools it owns. This is
//! `aivyx-contacts`'s sweep: each tool's name / description /
//! schema — the metadata the LLM relies on to call it — must
//! meet the correctness floor.

use std::path::PathBuf;
use std::sync::Arc;

use aivyx_contacts::tools::{
    ContactsCreate, ContactsDelete, ContactsGet, ContactsList, ContactsSearch, ContactsUpdate,
};
use aivyx_contacts::ContactsClient;
use aivyx_core::tools::check_tool_quality;
use aivyx_core::Tool;
use aivyx_google_oauth::{OAuthConfig, TokenSet};

fn fake_client() -> Arc<ContactsClient> {
    let config = OAuthConfig::new(
        "test-client-id",
        "test-client-secret",
        "http://127.0.0.1:0/callback",
    )
    .with_scopes(["https://www.googleapis.com/auth/contacts"]);
    let tokens = TokenSet {
        access_token: "fake-access-token".to_string(),
        refresh_token: Some("fake-refresh-token".to_string()),
        expires_at_unix_secs: 0,
        granted_scope: "https://www.googleapis.com/auth/contacts".to_string(),
        token_type: "Bearer".to_string(),
    };
    Arc::new(ContactsClient::new(
        reqwest::Client::new(),
        config,
        tokens,
        PathBuf::from("/tmp/aivyx-contacts-tool-quality-test-tokens.json"),
    ))
}

#[test]
fn contacts_tools_meet_quality_floor() {
    let client = fake_client();
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ContactsSearch::new(client.clone())),
        Arc::new(ContactsList::new(client.clone())),
        Arc::new(ContactsGet::new(client.clone())),
        Arc::new(ContactsCreate::new(client.clone())),
        Arc::new(ContactsUpdate::new(client.clone())),
        Arc::new(ContactsDelete::new(client)),
    ];

    let issues: Vec<String> = tools
        .iter()
        .flat_map(|t| check_tool_quality(t.as_ref()))
        .collect();

    assert!(issues.is_empty(), "contacts tool quality issues:\n  {}", issues.join("\n  "));
}
