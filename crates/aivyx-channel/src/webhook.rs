//! Webhook trigger primitive — Phase 27 Task 3.
//!
//! A webhook is an HTTP-triggered execution entry that creates daemon
//! turns when a `POST /trigger/<id>` request arrives. Backed by a redb
//! row under `KeyDomain::Webhooks`. Follows the same CRUD pattern as
//! `schedule.rs`.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use aivyx_storage::{DomainHandle, KeyDomain, StorageError};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookRecord {
    pub webhook_id: String,
    pub role_name: String,
    pub prompt: String,
    pub enabled: bool,
    pub created_at: u64,
    pub last_fired_at: Option<u64>,
}

impl WebhookRecord {
    pub fn new(
        webhook_id: String,
        role_name: String,
        prompt: String,
    ) -> Self {
        WebhookRecord {
            webhook_id,
            role_name,
            prompt,
            enabled: true,
            created_at: now_millis(),
            last_fired_at: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Storage CRUD
// ---------------------------------------------------------------------------

fn webhook_key(webhook_id: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(webhook_id.len());
    key.extend_from_slice(webhook_id.as_bytes());
    key
}

pub async fn create_webhook(
    handle: &DomainHandle,
    record: &WebhookRecord,
) -> Result<(), StorageError> {
    assert_eq!(handle.domain(), KeyDomain::Webhooks);
    let json = serde_json::to_vec(record).map_err(|e| {
        StorageError::Redb(format!("serialize WebhookRecord: {e}"))
    })?;
    handle.put(&webhook_key(&record.webhook_id), &json).await
}

pub async fn get_webhook(
    handle: &DomainHandle,
    webhook_id: &str,
) -> Result<Option<WebhookRecord>, StorageError> {
    assert_eq!(handle.domain(), KeyDomain::Webhooks);
    match handle.get(&webhook_key(webhook_id)).await? {
        Some(bytes) => {
            let record: WebhookRecord =
                serde_json::from_slice(&bytes).map_err(|e| {
                    StorageError::Redb(format!("deserialize WebhookRecord: {e}"))
                })?;
            Ok(Some(record))
        }
        None => Ok(None),
    }
}

pub async fn update_webhook(
    handle: &DomainHandle,
    record: &WebhookRecord,
) -> Result<(), StorageError> {
    create_webhook(handle, record).await
}

pub async fn list_webhooks(
    handle: &DomainHandle,
) -> Result<Vec<WebhookRecord>, StorageError> {
    assert_eq!(handle.domain(), KeyDomain::Webhooks);
    let rows = handle.scan_prefix(b"").await?;
    let mut webhooks = Vec::with_capacity(rows.len());
    for (_key, bytes) in rows {
        let record: WebhookRecord =
            serde_json::from_slice(&bytes).map_err(|e| {
                StorageError::Redb(format!("deserialize WebhookRecord: {e}"))
            })?;
        webhooks.push(record);
    }
    Ok(webhooks)
}

pub async fn delete_webhook(
    handle: &DomainHandle,
    webhook_id: &str,
) -> Result<(), StorageError> {
    assert_eq!(handle.domain(), KeyDomain::Webhooks);
    handle.delete(&webhook_key(webhook_id)).await
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_record_new_sets_defaults() {
        let r = WebhookRecord::new(
            "wh-1".into(),
            "default".into(),
            "handle webhook event".into(),
        );
        assert_eq!(r.webhook_id, "wh-1");
        assert_eq!(r.role_name, "default");
        assert_eq!(r.prompt, "handle webhook event");
        assert!(r.enabled);
        assert!(r.last_fired_at.is_none());
        assert!(r.created_at > 0);
    }

    #[test]
    fn webhook_record_round_trips_through_serde() {
        let record = WebhookRecord::new(
            "test-wh".into(),
            "ops".into(),
            "process incoming data".into(),
        );
        let json = serde_json::to_vec(&record).unwrap();
        let back: WebhookRecord = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.webhook_id, "test-wh");
        assert_eq!(back.role_name, "ops");
        assert_eq!(back.prompt, "process incoming data");
        assert!(back.enabled);
        assert!(back.last_fired_at.is_none());
    }

    #[test]
    fn webhook_key_is_stable() {
        let k = webhook_key("my-webhook");
        assert_eq!(k, b"my-webhook");
    }
}
