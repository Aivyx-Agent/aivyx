//! MCP protocol message types — typed wrappers for the methods we use:
//! `initialize`, `tools/list`, `tools/call`, and (capability-gated)
//! `resources/list`, `resources/read`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
}

#[derive(Debug, Serialize)]
pub struct ClientCapabilities {}

#[derive(Debug, Serialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Protocol struct — fields parsed for validation
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(rename = "serverInfo")]
    pub server_info: Option<ServerInfo>,
    pub capabilities: Option<ServerCapabilities>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Protocol struct — fields parsed for validation
pub struct ServerInfo {
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // Protocol struct — fields parsed for capability gating
pub struct ServerCapabilities {
    /// Present when the server exposes tools (`tools/list`, `tools/call`).
    pub tools: Option<Value>,
    /// Present when the server exposes resources (`resources/list`,
    /// `resources/read`). The client only probes resources when this is
    /// declared.
    #[serde(default)]
    pub resources: Option<Value>,
    /// Present when the server exposes prompts (`prompts/list`,
    /// `prompts/get`). Probed only when declared.
    #[serde(default)]
    pub prompts: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpToolDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Deserialize)]
pub struct ToolsListResult {
    pub tools: Vec<McpToolDef>,
}

#[derive(Debug, Serialize)]
pub struct ToolsCallParams {
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Deserialize)]
pub struct ToolsCallResult {
    pub content: Vec<ContentBlock>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

#[derive(Debug, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
}

// ---- resources (capability-gated) ----------------------------------

/// A resource the MCP server exposes (an entry from `resources/list`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceDef {
    pub uri: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "mimeType", default)]
    pub mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResourcesListResult {
    #[serde(default)]
    pub resources: Vec<McpResourceDef>,
}

#[derive(Debug, Serialize)]
pub struct ResourcesReadParams {
    pub uri: String,
}

/// One content item from `resources/read`. Text resources carry `text`;
/// binary resources carry a base64 `blob`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceContents {
    pub uri: String,
    #[serde(rename = "mimeType", default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub blob: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResourcesReadResult {
    #[serde(default)]
    pub contents: Vec<ResourceContents>,
}

// ---- prompts (capability-gated) ------------------------------------

/// One argument a prompt template accepts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
}

/// A prompt the MCP server exposes (an entry from `prompts/list`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub arguments: Vec<PromptArgument>,
}

#[derive(Debug, Deserialize)]
pub struct PromptsListResult {
    #[serde(default)]
    pub prompts: Vec<McpPromptDef>,
}

#[derive(Debug, Serialize)]
pub struct PromptsGetParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
}

/// One rendered message from `prompts/get` (role + content).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptMessage {
    pub role: String,
    pub content: PromptContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PromptsGetResult {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub messages: Vec<PromptMessage>,
}
