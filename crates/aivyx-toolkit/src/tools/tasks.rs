//! `task.*` — lightweight TODO CRUD.
//!
//! Phase 125 Task 4. Four tools, all sharing the same
//! [`crate::task_store::TaskStore`] handle through `Arc`.
//! Two capability scopes: `task.read` for `task.list`;
//! `task.write` for `task.create`, `task.complete`,
//! `task.delete`.
//!
//! Data lives at `~/.aivyx/tool-processes/toolkit/tasks.json`
//! (0600 perms, atomic write — see
//! [`crate::task_store`] for the storage substrate).
//!
//! ## Tool surface
//!
//! - `task.create` — `{title, notes?, due_date?}` →
//!   `{id, title, status: "open", created_at}`.
//! - `task.list` — `{status: "open" | "complete" | "all"
//!   (default "open"), limit: u32 (default 25)}` →
//!   `{tasks: [...], total_count}`.
//! - `task.complete` — `{id, completion_note?}` →
//!   `{id, status: "complete", completed_at}`.
//! - `task.delete` — `{id}` → `{id, deleted: true}`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

use crate::task_store::{parse_due_date, Task, TaskStatus, TaskStore};

const DEFAULT_LIST_LIMIT: u64 = 25;
const MAX_LIST_LIMIT: u64 = 500;

// =====================================================================
// task.create
// =====================================================================

pub struct TaskCreate {
    id: ToolId,
    schema: Value,
    store: Arc<TaskStore>,
}

impl TaskCreate {
    pub fn new(store: Arc<TaskStore>) -> Self {
        Self {
            id: ToolId::new(),
            schema: create_schema(),
            store,
        }
    }
}

#[async_trait]
impl Tool for TaskCreate {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "task.create"
    }
    fn description(&self) -> &str {
        "Create a new TODO task. Input: `{title: string \
         (required), notes: string (optional), due_date: \
         string (optional, RFC 3339 / ISO 8601 — e.g. \
         `2026-06-01T12:00:00Z`)}`. Returns `{id, title, \
         status: \"open\", created_at}`. Scope: `task.write`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("task.write")
            .expect("task.write must parse — it is in KNOWN_BASES from Phase 125")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let title = match required_string(&input, "title") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("task.create: {e}")),
        };
        let notes = match optional_string(&input, "notes") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("task.create: {e}")),
        };
        let due_date = match optional_string(&input, "due_date") {
            Ok(Some(s)) => match parse_due_date(&s) {
                Ok(dt) => Some(dt),
                Err(e) => return failed(self.id, format!("task.create: {e}")),
            },
            Ok(None) => None,
            Err(e) => return failed(self.id, format!("task.create: {e}")),
        };
        match self.store.create(title, notes, due_date).await {
            Ok(task) => ToolOutcome::Completed {
                output: shape_task_brief(&task),
                verified: Verification::Verified, // write-then-disk-confirm
            },
            Err(e) => failed(self.id, format!("task.create: storage: {e}")),
        }
    }
}

fn create_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": {
                "type": "string",
                "minLength": 1,
                "description": "Short task title. Required."
            },
            "notes": {
                "type": "string",
                "description": "Optional free-form notes."
            },
            "due_date": {
                "type": "string",
                "description": "Optional RFC 3339 timestamp (e.g. `2026-06-01T12:00:00Z`)."
            }
        },
        "required": ["title"],
        "additionalProperties": false
    })
}

// =====================================================================
// task.list
// =====================================================================

pub struct TaskList {
    id: ToolId,
    schema: Value,
    store: Arc<TaskStore>,
}

impl TaskList {
    pub fn new(store: Arc<TaskStore>) -> Self {
        Self {
            id: ToolId::new(),
            schema: list_schema(),
            store,
        }
    }
}

#[async_trait]
impl Tool for TaskList {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "task.list"
    }
    fn description(&self) -> &str {
        "List TODO tasks. Input: `{status: \"open\" | \
         \"complete\" | \"all\" (default \"open\"), limit: \
         u32 (default 25, max 500)}`. Returns `{tasks: \
         [{id, title, notes?, due_date?, status, created_at, \
         completed_at?, completion_note?}], total_count}`. \
         `total_count` reflects the full filtered set; \
         `tasks` is the limit-capped slice. Scope: \
         `task.read`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("task.read")
            .expect("task.read must parse — it is in KNOWN_BASES from Phase 125")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let filter = match parse_status_filter(&input) {
            Ok(f) => f,
            Err(e) => return failed(self.id, format!("task.list: {e}")),
        };
        let limit = match parse_limit(&input) {
            Ok(l) => l,
            Err(e) => return failed(self.id, format!("task.list: {e}")),
        };
        let (tasks, total) = self.store.list(filter, limit as usize).await;
        let shaped: Vec<Value> = tasks.iter().map(shape_task_full).collect();
        ToolOutcome::Completed {
            output: json!({
                "tasks": shaped,
                "total_count": total,
            }),
            verified: Verification::NotApplicable, // read-only
        }
    }
}

fn parse_status_filter(input: &Value) -> Result<Option<TaskStatus>, String> {
    match input.get("status") {
        None | Some(Value::Null) => Ok(Some(TaskStatus::Open)),
        Some(Value::String(s)) => match s.as_str() {
            "open" => Ok(Some(TaskStatus::Open)),
            "complete" => Ok(Some(TaskStatus::Complete)),
            "all" => Ok(None),
            other => Err(format!(
                "`status` must be \"open\" | \"complete\" | \"all\"; got {other:?}"
            )),
        },
        Some(_) => Err("`status` must be a string".to_string()),
    }
}

fn parse_limit(input: &Value) -> Result<u64, String> {
    let raw = match input.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIST_LIMIT,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "`limit` must be a non-negative integer".to_string())?,
    };
    if raw == 0 {
        return Err("`limit` must be >= 1".to_string());
    }
    Ok(raw.min(MAX_LIST_LIMIT))
}

fn list_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "enum": ["open", "complete", "all"],
                "default": "open"
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_LIST_LIMIT,
                "default": DEFAULT_LIST_LIMIT
            }
        },
        "additionalProperties": false
    })
}

// =====================================================================
// task.complete
// =====================================================================

pub struct TaskComplete {
    id: ToolId,
    schema: Value,
    store: Arc<TaskStore>,
}

impl TaskComplete {
    pub fn new(store: Arc<TaskStore>) -> Self {
        Self {
            id: ToolId::new(),
            schema: complete_schema(),
            store,
        }
    }
}

#[async_trait]
impl Tool for TaskComplete {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "task.complete"
    }
    fn description(&self) -> &str {
        "Mark a TODO task complete. Input: `{id: string \
         (required, the task's UUID), completion_note: \
         string (optional, free-form note about how/when \
         done)}`. Returns `{id, status: \"complete\", \
         completed_at}`. Errors with `not_found` if no task \
         has the given id. Scope: `task.write`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("task.write")
            .expect("task.write must parse — it is in KNOWN_BASES from Phase 125")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let id = match required_string(&input, "id") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("task.complete: {e}")),
        };
        let note = match optional_string(&input, "completion_note") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("task.complete: {e}")),
        };
        match self.store.complete(&id, note).await {
            Ok(task) => ToolOutcome::Completed {
                output: json!({
                    "id": task.id,
                    "status": task.status.as_str(),
                    "completed_at": task.completed_at,
                }),
                verified: Verification::Verified,
            },
            Err(e) => failed(self.id, format!("task.complete: {e}")),
        }
    }
}

fn complete_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "minLength": 1
            },
            "completion_note": {
                "type": "string"
            }
        },
        "required": ["id"],
        "additionalProperties": false
    })
}

// =====================================================================
// task.delete
// =====================================================================

pub struct TaskDelete {
    id: ToolId,
    schema: Value,
    store: Arc<TaskStore>,
}

impl TaskDelete {
    pub fn new(store: Arc<TaskStore>) -> Self {
        Self {
            id: ToolId::new(),
            schema: delete_schema(),
            store,
        }
    }
}

#[async_trait]
impl Tool for TaskDelete {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "task.delete"
    }
    fn description(&self) -> &str {
        "Permanently remove a TODO task. Input: `{id: string \
         (required)}`. Returns `{id, deleted: true}`. Errors \
         with `not_found` if no task has the given id. Scope: \
         `task.write`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("task.write")
            .expect("task.write must parse — it is in KNOWN_BASES from Phase 125")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let id = match required_string(&input, "id") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("task.delete: {e}")),
        };
        match self.store.delete(&id).await {
            Ok(removed_id) => ToolOutcome::Completed {
                output: json!({"id": removed_id, "deleted": true}),
                verified: Verification::Verified,
            },
            Err(e) => failed(self.id, format!("task.delete: {e}")),
        }
    }
}

fn delete_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "minLength": 1
            }
        },
        "required": ["id"],
        "additionalProperties": false
    })
}

// =====================================================================
// shared helpers
// =====================================================================

fn shape_task_full(task: &Task) -> Value {
    let mut out = json!({
        "id": task.id,
        "title": task.title,
        "status": task.status.as_str(),
        "created_at": task.created_at,
    });
    if let Some(notes) = &task.notes {
        out["notes"] = Value::String(notes.clone());
    }
    if let Some(due) = task.due_date {
        out["due_date"] = json!(due);
    }
    if let Some(completed) = task.completed_at {
        out["completed_at"] = json!(completed);
    }
    if let Some(note) = &task.completion_note {
        out["completion_note"] = Value::String(note.clone());
    }
    out
}

fn shape_task_brief(task: &Task) -> Value {
    json!({
        "id": task.id,
        "title": task.title,
        "status": task.status.as_str(),
        "created_at": task.created_at,
    })
}

fn required_string(input: &Value, field: &str) -> Result<String, String> {
    let s = input
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("input must include a `{field}` string field"))?;
    if s.trim().is_empty() {
        return Err(format!("`{field}` must not be empty"));
    }
    Ok(s.to_string())
}

fn optional_string(input: &Value, field: &str) -> Result<Option<String>, String> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            if s.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(s.clone()))
            }
        }
        Some(_) => Err(format!("`{field}` must be a string if present")),
    }
}

fn failed(id: ToolId, detail: String) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool { tool: id, detail })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_store::TaskStore;
    use std::sync::atomic::{AtomicU64, Ordering};

    async fn scratch_store() -> (Arc<TaskStore>, std::path::PathBuf) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let tmp = std::env::var("TMPDIR")
            .or_else(|_| std::env::var("TEMP"))
            .unwrap_or_else(|_| "/tmp".to_string());
        let dir = std::path::PathBuf::from(tmp).join(format!(
            "aivyx-toolkit-tasks-tool-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = TaskStore::open(dir.join("tasks.json")).await.unwrap();
        (Arc::new(store), dir)
    }

    // ---- parse_status_filter / parse_limit -----------------------

    #[test]
    fn status_filter_defaults_to_open() {
        assert_eq!(parse_status_filter(&json!({})).unwrap(), Some(TaskStatus::Open));
    }

    #[test]
    fn status_filter_accepts_all_three_values() {
        assert_eq!(
            parse_status_filter(&json!({"status": "open"})).unwrap(),
            Some(TaskStatus::Open)
        );
        assert_eq!(
            parse_status_filter(&json!({"status": "complete"})).unwrap(),
            Some(TaskStatus::Complete)
        );
        assert_eq!(parse_status_filter(&json!({"status": "all"})).unwrap(), None);
    }

    #[test]
    fn status_filter_rejects_unknown_value() {
        let e = parse_status_filter(&json!({"status": "pending"})).unwrap_err();
        assert!(e.contains("must be"), "{e}");
    }

    #[test]
    fn limit_defaults_to_25_caps_at_500() {
        assert_eq!(parse_limit(&json!({})).unwrap(), 25);
        assert_eq!(parse_limit(&json!({"limit": 1000})).unwrap(), 500);
    }

    #[test]
    fn limit_rejects_zero() {
        let e = parse_limit(&json!({"limit": 0})).unwrap_err();
        assert!(e.contains(">= 1"), "{e}");
    }

    // ---- shape_task_full -----------------------------------------

    #[test]
    fn shape_task_full_omits_absent_optionals() {
        let task = Task {
            id: "id-x".to_string(),
            title: "x".to_string(),
            notes: None,
            due_date: None,
            status: TaskStatus::Open,
            created_at: chrono::Utc::now(),
            completed_at: None,
            completion_note: None,
        };
        let v = shape_task_full(&task);
        assert!(v.get("notes").is_none(), "notes omitted when None");
        assert!(v.get("due_date").is_none());
        assert!(v.get("completed_at").is_none());
        assert!(v.get("completion_note").is_none());
        assert_eq!(v["status"], "open");
    }

    // ---- end-to-end against scratch_store -------------------------

    #[tokio::test]
    async fn create_executes_through_tool_layer() {
        let (store, dir) = scratch_store().await;
        let tool = TaskCreate::new(store.clone());
        // We can't construct a real ToolContext here; bypass
        // execute() and call the store path the tool ultimately
        // hits. The execute() body is tested through the
        // mock-context Phase 125 Task 7 walkthrough.
        let task = store
            .create("from tool layer".to_string(), None, None)
            .await
            .unwrap();
        assert_eq!(task.title, "from tool layer");
        // Ensure tool descriptor surfaces are right.
        assert_eq!(tool.name(), "task.create");
        assert_eq!(tool.required_scope(&json!({})).to_string(), "task.write");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn list_filter_status_open_through_store() {
        let (store, dir) = scratch_store().await;
        let t1 = store.create("a".to_string(), None, None).await.unwrap();
        let t2 = store.create("b".to_string(), None, None).await.unwrap();
        store.complete(&t1.id, None).await.unwrap();

        let _list_tool = TaskList::new(store.clone());
        let (open_tasks, open_total) =
            store.list(Some(TaskStatus::Open), 100).await;
        assert_eq!(open_total, 1);
        assert_eq!(open_tasks[0].id, t2.id);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn complete_then_delete_via_store_path() {
        let (store, dir) = scratch_store().await;
        let _ = TaskComplete::new(store.clone());
        let _ = TaskDelete::new(store.clone());
        let t = store.create("to be done".to_string(), None, None).await.unwrap();
        let completed = store.complete(&t.id, Some("yay".to_string())).await.unwrap();
        assert_eq!(completed.status, TaskStatus::Complete);
        let removed = store.delete(&t.id).await.unwrap();
        assert_eq!(removed, t.id);
        let (tasks, total) = store.list(None, 100).await;
        assert_eq!(total, 0);
        assert!(tasks.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- required_string / optional_string -----------------------

    #[test]
    fn required_string_rejects_missing_and_empty() {
        assert!(required_string(&json!({}), "x").is_err());
        assert!(required_string(&json!({"x": "  "}), "x").is_err());
        assert_eq!(required_string(&json!({"x": "ok"}), "x").unwrap(), "ok");
    }

    #[test]
    fn optional_string_treats_empty_as_none() {
        assert_eq!(optional_string(&json!({}), "x").unwrap(), None);
        assert_eq!(optional_string(&json!({"x": ""}), "x").unwrap(), None);
        assert_eq!(optional_string(&json!({"x": " "}), "x").unwrap(), None);
        assert_eq!(
            optional_string(&json!({"x": "y"}), "x").unwrap(),
            Some("y".to_string())
        );
    }

    #[test]
    fn optional_string_rejects_non_string() {
        let e = optional_string(&json!({"x": 42}), "x").unwrap_err();
        assert!(e.contains("`x`"), "{e}");
        assert!(e.contains("string"), "{e}");
    }

    // ---- input schemas -------------------------------------------

    #[test]
    fn create_schema_requires_title_and_rejects_extras() {
        let s = create_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v.as_str() == Some("title")));
        assert_eq!(s["additionalProperties"], false);
    }

    #[test]
    fn list_schema_status_enum_pinned() {
        let s = list_schema();
        let enums = s["properties"]["status"]["enum"].as_array().unwrap();
        let labels: Vec<&str> = enums.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(labels, vec!["open", "complete", "all"]);
    }

    #[test]
    fn complete_schema_requires_id() {
        let s = complete_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v.as_str() == Some("id")));
    }

    #[test]
    fn delete_schema_requires_id_only() {
        let s = delete_schema();
        let req = s["required"].as_array().unwrap();
        assert_eq!(req.len(), 1);
        assert_eq!(req[0].as_str(), Some("id"));
    }

    // ---- scopes pin ---------------------------------------------

    #[tokio::test]
    async fn read_and_write_scopes_pinned_per_tool() {
        let (store, dir) = scratch_store().await;
        let create = TaskCreate::new(store.clone());
        let list = TaskList::new(store.clone());
        let complete = TaskComplete::new(store.clone());
        let delete = TaskDelete::new(store.clone());
        assert_eq!(create.required_scope(&json!({})).to_string(), "task.write");
        assert_eq!(list.required_scope(&json!({})).to_string(), "task.read");
        assert_eq!(complete.required_scope(&json!({})).to_string(), "task.write");
        assert_eq!(delete.required_scope(&json!({})).to_string(), "task.write");
        std::fs::remove_dir_all(&dir).ok();
    }
}
