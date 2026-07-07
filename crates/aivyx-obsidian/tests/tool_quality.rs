//! Atlas (AT.3) recommended every tool-process crate run
//! `check_tool_quality` over the tools it owns. This is
//! `aivyx-obsidian`'s sweep: each tool's name / description /
//! schema — the metadata the LLM relies on to call it — must
//! meet the correctness floor.

use std::sync::Arc;

use aivyx_core::tools::check_tool_quality;
use aivyx_core::Tool;
use aivyx_obsidian::tools::{
    ObsidianCreateNote, ObsidianDeleteNote, ObsidianGetNote, ObsidianListFolder, ObsidianSearch,
    ObsidianUpdateNote,
};
use aivyx_obsidian::{VaultClient, VaultConfig};

/// `VaultClient::new` canonicalizes + requires an existing
/// directory, so the fixture needs a real (empty) temp vault —
/// mirrors the `tmp_vault` pattern in `tools/search.rs`'s own
/// unit tests.
fn fake_client() -> Arc<VaultClient> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "aivyx-obsidian-tool-quality-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));
    std::fs::create_dir_all(&path).expect("create temp vault dir");
    let config = VaultConfig { vault_path: path };
    Arc::new(VaultClient::new(config).expect("vault client over a real temp dir"))
}

#[test]
fn obsidian_tools_meet_quality_floor() {
    let client = fake_client();
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ObsidianSearch::new(client.clone())),
        Arc::new(ObsidianGetNote::new(client.clone())),
        Arc::new(ObsidianListFolder::new(client.clone())),
        Arc::new(ObsidianCreateNote::new(client.clone())),
        Arc::new(ObsidianUpdateNote::new(client.clone())),
        Arc::new(ObsidianDeleteNote::new(client)),
    ];

    let issues: Vec<String> = tools
        .iter()
        .flat_map(|t| check_tool_quality(t.as_ref()))
        .collect();

    assert!(issues.is_empty(), "obsidian tool quality issues:\n  {}", issues.join("\n  "));
}
