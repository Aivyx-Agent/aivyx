//! Atlas (AT.3) recommended every tool-process crate run
//! `check_tool_quality` over the tools it owns. This is
//! `aivyx-notion`'s sweep: each tool's name / description /
//! schema — the metadata the LLM relies on to call it — must
//! meet the correctness floor.

use std::sync::Arc;

use aivyx_core::tools::check_tool_quality;
use aivyx_core::Tool;
use aivyx_notion::tools::{
    NotionAppendBlocks, NotionArchivePage, NotionCreatePage, NotionGetPage, NotionListDatabase,
    NotionSearch, NotionUpdatePageProperties,
};
use aivyx_notion::{NotionClient, NotionConfig};

fn fake_client() -> Arc<NotionClient> {
    let config = NotionConfig::new("ntn_fake-test-token");
    Arc::new(NotionClient::new(reqwest::Client::new(), config))
}

#[test]
fn notion_tools_meet_quality_floor() {
    let client = fake_client();
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(NotionSearch::new(client.clone())),
        Arc::new(NotionGetPage::new(client.clone())),
        Arc::new(NotionListDatabase::new(client.clone())),
        Arc::new(NotionCreatePage::new(client.clone())),
        Arc::new(NotionUpdatePageProperties::new(client.clone())),
        Arc::new(NotionAppendBlocks::new(client.clone())),
        Arc::new(NotionArchivePage::new(client)),
    ];

    let issues: Vec<String> = tools
        .iter()
        .flat_map(|t| check_tool_quality(t.as_ref()))
        .collect();

    assert!(issues.is_empty(), "notion tool quality issues:\n  {}", issues.join("\n  "));
}
