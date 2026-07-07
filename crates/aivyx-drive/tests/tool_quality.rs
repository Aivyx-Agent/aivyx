//! Atlas (AT.3) recommended every tool-process crate run
//! `check_tool_quality` over the tools it owns. This is
//! `aivyx-drive`'s sweep: each tool's name / description /
//! schema — the metadata the LLM relies on to call it — must
//! meet the correctness floor.

use std::path::PathBuf;
use std::sync::Arc;

use aivyx_core::tools::check_tool_quality;
use aivyx_core::Tool;
use aivyx_drive::tools::{
    DriveCreateFolder, DriveDeleteFile, DriveDownloadFile, DriveGetMetadata, DriveListDrives,
    DriveListFolder, DriveRecentActivity, DriveRecentChanges, DriveRecentFiles, DriveSearch,
    DriveUploadFile,
};
use aivyx_drive::DriveClient;
use aivyx_google_oauth::{OAuthConfig, TokenSet};

fn fake_client() -> Arc<DriveClient> {
    let config = OAuthConfig::new(
        "test-client-id",
        "test-client-secret",
        "http://127.0.0.1:0/callback",
    )
    .with_scopes(["https://www.googleapis.com/auth/drive"]);
    let tokens = TokenSet {
        access_token: "fake-access-token".to_string(),
        refresh_token: Some("fake-refresh-token".to_string()),
        expires_at_unix_secs: 0,
        granted_scope: "https://www.googleapis.com/auth/drive".to_string(),
        token_type: "Bearer".to_string(),
    };
    Arc::new(DriveClient::new(
        reqwest::Client::new(),
        config,
        tokens,
        PathBuf::from("/tmp/aivyx-drive-tool-quality-test-tokens.json"),
    ))
}

#[test]
fn drive_tools_meet_quality_floor() {
    let client = fake_client();
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(DriveSearch::new(client.clone())),
        Arc::new(DriveGetMetadata::new(client.clone())),
        Arc::new(DriveListFolder::new(client.clone())),
        Arc::new(DriveDownloadFile::new(client.clone())),
        Arc::new(DriveCreateFolder::new(client.clone())),
        Arc::new(DriveUploadFile::new(client.clone())),
        Arc::new(DriveDeleteFile::new(client.clone())),
        Arc::new(DriveListDrives::new(client.clone())),
        Arc::new(DriveRecentFiles::new(client.clone())),
        Arc::new(DriveRecentChanges::new(client.clone())),
        Arc::new(DriveRecentActivity::new(client)),
    ];

    let issues: Vec<String> = tools
        .iter()
        .flat_map(|t| check_tool_quality(t.as_ref()))
        .collect();

    assert!(issues.is_empty(), "drive tool quality issues:\n  {}", issues.join("\n  "));
}
