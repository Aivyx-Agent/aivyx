//! Atlas (AT.3) recommended every tool-process crate run
//! `check_tool_quality` over the tools it owns. This is
//! `aivyx-n8n`'s sweep: each tool's name / description /
//! schema — the metadata the LLM relies on to call it — must
//! meet the correctness floor.

use std::sync::Arc;

use aivyx_core::tools::check_tool_quality;
use aivyx_core::Tool;
use aivyx_n8n::tools::{
    N8nActivateWorkflow, N8nCreateWorkflow, N8nDeactivateWorkflow, N8nDeleteWorkflow,
    N8nExecuteWorkflow, N8nGetExecution, N8nGetWorkflow, N8nListExecutions, N8nListWorkflows,
    N8nUpdateWorkflow,
};
use aivyx_n8n::{N8nClient, N8nConfig};

fn fake_client() -> Arc<N8nClient> {
    let config = N8nConfig {
        n8n_base_url: "http://127.0.0.1:0".to_string(),
        n8n_api_key: "fake-test-api-key".to_string(),
    };
    Arc::new(N8nClient::new(reqwest::Client::new(), config))
}

#[test]
fn n8n_tools_meet_quality_floor() {
    let client = fake_client();
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(N8nListWorkflows::new(client.clone())),
        Arc::new(N8nGetWorkflow::new(client.clone())),
        Arc::new(N8nCreateWorkflow::new(client.clone())),
        Arc::new(N8nUpdateWorkflow::new(client.clone())),
        Arc::new(N8nDeleteWorkflow::new(client.clone())),
        Arc::new(N8nActivateWorkflow::new(client.clone())),
        Arc::new(N8nDeactivateWorkflow::new(client.clone())),
        Arc::new(N8nExecuteWorkflow::new(client.clone())),
        Arc::new(N8nListExecutions::new(client.clone())),
        Arc::new(N8nGetExecution::new(client)),
    ];

    let issues: Vec<String> = tools
        .iter()
        .flat_map(|t| check_tool_quality(t.as_ref()))
        .collect();

    assert!(issues.is_empty(), "n8n tool quality issues:\n  {}", issues.join("\n  "));
}
