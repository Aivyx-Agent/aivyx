//! Chapter Deckhand — the `app.*` tool surface.
//!
//! Six tools across three capability bases:
//! - `app.read`    → [`AppList`], [`AppScreenshot`] (observe)
//! - `app.control` → [`AppFocus`] (raise a window — reversible)
//! - `app.input`   → [`AppType`], [`AppKey`], [`AppClick`] (inject input —
//!   irreversible, confirm-first via `IRREVERSIBLE_BASES`)
//!
//! Each tool is a thin wrapper over [`crate::backend`]: validate input, build
//! the xdotool argv (pure, tested in `backend`), run it, shape the result.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::backend;

fn failed(id: ToolId, detail: String) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool { tool: id, detail })
}

fn required_string(input: &Value, field: &str) -> Result<String, String> {
    input
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| format!("missing required string field `{field}`"))
}

fn scope(base: &str) -> Scope {
    Scope::parse(base)
        .unwrap_or_else(|| panic!("{base} must parse — in KNOWN_BASES (Chapter Deckhand)"))
}

// ---- app.read -----------------------------------------------------------

/// `app.list` — enumerate the open (visible) windows.
pub struct AppList {
    id: ToolId,
    schema: Value,
}
impl Default for AppList {
    fn default() -> Self {
        Self::new()
    }
}
impl AppList {
    pub fn new() -> Self {
        Self { id: ToolId::new(), schema: json!({"type": "object", "properties": {}}) }
    }
}
#[async_trait]
impl Tool for AppList {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "app.list"
    }
    fn description(&self) -> &str {
        "List the open windows on the operator's desktop. No input. Returns \
         `{windows: [{id, title, active}]}` — `id` is an X11 window id you pass \
         to `app.focus`. Linux/X11 + Xwayland only (native Wayland windows are \
         not visible). Scope: `app.read`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        scope("app.read")
    }
    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        match backend::list_windows().await {
            Ok(windows) => ToolOutcome::Completed {
                output: json!({ "windows": windows }),
                verified: Verification::Unverified,
            },
            Err(e) => failed(self.id, format!("app.list: {e}")),
        }
    }
}

/// `app.screenshot` — capture the screen to a PNG file.
pub struct AppScreenshot {
    id: ToolId,
    schema: Value,
}
impl Default for AppScreenshot {
    fn default() -> Self {
        Self::new()
    }
}
impl AppScreenshot {
    pub fn new() -> Self {
        Self { id: ToolId::new(), schema: json!({"type": "object", "properties": {}}) }
    }
}
#[async_trait]
impl Tool for AppScreenshot {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "app.screenshot"
    }
    fn description(&self) -> &str {
        "Capture a full-screen screenshot to a PNG file and return its path. \
         No input. Returns `{path, tool}`. Use a vision-capable model (or \
         `data`/`fs.read`) to interpret the image. Requires a screenshot tool \
         (grim / maim / scrot / import). Scope: `app.read`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        scope("app.read")
    }
    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("aivyx-apps-shot-{ts}.png"));
        match backend::capture_screenshot(&path).await {
            Ok(tool) => ToolOutcome::Completed {
                output: json!({ "path": path.to_string_lossy(), "tool": tool }),
                verified: Verification::Unverified,
            },
            Err(e) => failed(self.id, format!("app.screenshot: {e}")),
        }
    }
}

// ---- app.control --------------------------------------------------------

/// `app.focus` — raise/focus a window by id (reversible).
pub struct AppFocus {
    id: ToolId,
    schema: Value,
}
impl Default for AppFocus {
    fn default() -> Self {
        Self::new()
    }
}
impl AppFocus {
    pub fn new() -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": { "window_id": {"type": "string"} },
                "required": ["window_id"]
            }),
        }
    }
}
#[async_trait]
impl Tool for AppFocus {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "app.focus"
    }
    fn description(&self) -> &str {
        "Raise and focus a window. Input: `{window_id: string}` (a numeric id \
         from app.list). Returns `{ok, window_id}`. Scope: `app.control`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        scope("app.control")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let window_id = match required_string(&input, "window_id") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("app.focus: {e}")),
        };
        let argv = match backend::focus_argv(&window_id) {
            Ok(a) => a,
            Err(e) => return failed(self.id, format!("app.focus: {e}")),
        };
        match backend::run_xdotool(&argv).await {
            Ok(_) => ToolOutcome::Completed {
                output: json!({ "ok": true, "window_id": window_id }),
                verified: Verification::Unverified,
            },
            Err(e) => failed(self.id, format!("app.focus: {e}")),
        }
    }
}

// ---- app.input (confirm-first) ------------------------------------------

/// `app.type` — type literal text into the focused window.
pub struct AppType {
    id: ToolId,
    schema: Value,
}
impl Default for AppType {
    fn default() -> Self {
        Self::new()
    }
}
impl AppType {
    pub fn new() -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": { "text": {"type": "string"} },
                "required": ["text"]
            }),
        }
    }
}
#[async_trait]
impl Tool for AppType {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "app.type"
    }
    fn description(&self) -> &str {
        "Type literal text into the currently-focused window. Input: \
         `{text: string}`. Returns `{ok, chars}`. Focus the target first with \
         app.focus. Confirm-first. Scope: `app.input`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        scope("app.input")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let text = match required_string(&input, "text") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("app.type: {e}")),
        };
        match backend::run_xdotool(&backend::type_argv(&text)).await {
            Ok(_) => ToolOutcome::Completed {
                output: json!({ "ok": true, "chars": text.chars().count() }),
                verified: Verification::Unverified,
            },
            Err(e) => failed(self.id, format!("app.type: {e}")),
        }
    }
}

/// `app.key` — send a key or chord to the focused window.
pub struct AppKey {
    id: ToolId,
    schema: Value,
}
impl Default for AppKey {
    fn default() -> Self {
        Self::new()
    }
}
impl AppKey {
    pub fn new() -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": { "keys": {"type": "string"} },
                "required": ["keys"]
            }),
        }
    }
}
#[async_trait]
impl Tool for AppKey {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "app.key"
    }
    fn description(&self) -> &str {
        "Send a key or chord to the focused window, e.g. \"ctrl+s\", \
         \"Return\", \"alt+Tab\". Input: `{keys: string}`. Returns `{ok, keys}`. \
         Confirm-first. Scope: `app.input`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        scope("app.input")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let keys = match required_string(&input, "keys") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("app.key: {e}")),
        };
        let argv = match backend::key_argv(&keys) {
            Ok(a) => a,
            Err(e) => return failed(self.id, format!("app.key: {e}")),
        };
        match backend::run_xdotool(&argv).await {
            Ok(_) => ToolOutcome::Completed {
                output: json!({ "ok": true, "keys": keys }),
                verified: Verification::Unverified,
            },
            Err(e) => failed(self.id, format!("app.key: {e}")),
        }
    }
}

/// `app.click` — click the mouse at a screen coordinate.
pub struct AppClick {
    id: ToolId,
    schema: Value,
}
impl Default for AppClick {
    fn default() -> Self {
        Self::new()
    }
}
impl AppClick {
    pub fn new() -> Self {
        Self {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "x": {"type": "integer"},
                    "y": {"type": "integer"},
                    "button": {"type": "integer", "description": "1=left (default), 2=middle, 3=right"}
                },
                "required": ["x", "y"]
            }),
        }
    }
}
#[async_trait]
impl Tool for AppClick {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "app.click"
    }
    fn description(&self) -> &str {
        "Move the mouse to (x, y) and click. Input: `{x: int, y: int, \
         button?: 1|2|3}` (default left). Returns `{ok, x, y, button}`. \
         Confirm-first. Scope: `app.input`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        scope("app.input")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let (Some(x), Some(y)) = (
            input.get("x").and_then(Value::as_i64),
            input.get("y").and_then(Value::as_i64),
        ) else {
            return failed(self.id, "app.click: `x` and `y` are required integers".into());
        };
        let button = input.get("button").and_then(Value::as_i64).unwrap_or(1);
        let button = if (1..=3).contains(&button) { button as u8 } else { 0 };
        let argv = match backend::click_argv(x, y, button) {
            Ok(a) => a,
            Err(e) => return failed(self.id, format!("app.click: {e}")),
        };
        match backend::run_xdotool(&argv).await {
            Ok(_) => ToolOutcome::Completed {
                output: json!({ "ok": true, "x": x, "y": y, "button": button }),
                verified: Verification::Unverified,
            },
            Err(e) => failed(self.id, format!("app.click: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_requires_its_declared_scope() {
        let empty = json!({});
        assert_eq!(AppList::new().required_scope(&empty).base(), "app.read");
        assert_eq!(AppScreenshot::new().required_scope(&empty).base(), "app.read");
        assert_eq!(AppFocus::new().required_scope(&empty).base(), "app.control");
        assert_eq!(AppType::new().required_scope(&empty).base(), "app.input");
        assert_eq!(AppKey::new().required_scope(&empty).base(), "app.input");
        assert_eq!(AppClick::new().required_scope(&empty).base(), "app.input");
    }

    #[test]
    fn required_string_rejects_missing_or_empty() {
        assert!(required_string(&json!({}), "text").is_err());
        assert!(required_string(&json!({"text": ""}), "text").is_err());
        assert_eq!(required_string(&json!({"text": "hi"}), "text").unwrap(), "hi");
    }
}
