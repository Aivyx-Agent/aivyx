//! File-watch daemon loop — Phase 27 Task 4.
//!
//! A background task that watches filesystem paths using the `notify`
//! crate and fires agent turns through `TriggerDispatch` when changes
//! are detected. Includes debounce logic to prevent rapid re-fires
//! from editor save storms.
//!
//! The watcher reloads the watch list from storage every 60 seconds to
//! pick up dynamically created watches (via `file_watch.create`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use aivyx_core::CancellationToken;
use aivyx_storage::DomainHandle;

use crate::file_watch::{self, FileWatchRecord};
use crate::trigger::{TriggerDispatch, TriggerSource};

const RELOAD_INTERVAL: Duration = Duration::from_secs(60);

/// Merge TOML `[[file_watch]]` entries into the file-watch store.
/// Entries whose `watch_id` already exists in storage are skipped
/// (storage is authoritative after first sync). Called once at daemon
/// startup.
pub async fn sync_config_file_watches(
    store: &DomainHandle,
    config_watches: &[FileWatchRecord],
) -> Result<usize, String> {
    let mut synced = 0;
    for record in config_watches {
        let existing = file_watch::get_file_watch(store, &record.watch_id)
            .await
            .map_err(|e| format!("sync_config_file_watches get: {e}"))?;
        if existing.is_none() {
            file_watch::create_file_watch(store, record)
                .await
                .map_err(|e| format!("sync_config_file_watches create: {e}"))?;
            synced += 1;
        }
    }
    Ok(synced)
}

/// Convert config-layer `FileWatchConfig` entries into storage-layer
/// `FileWatchRecord` values.
pub fn config_to_records(
    configs: &[aivyx_config::FileWatchConfig],
) -> Vec<FileWatchRecord> {
    configs
        .iter()
        .map(|c| {
            let mut r = FileWatchRecord::new(
                format!("cfg-{}", c.name),
                c.path.clone(),
                c.role.clone(),
                c.prompt.clone(),
            );
            r.enabled = c.enabled;
            r.wrap_mission = c.wrap_mission;
            if let Some(ms) = c.debounce_ms {
                r.debounce_ms = ms;
            }
            r.notify_target = c.notify_target.clone();
            r.notify_targets = c.notify_targets.clone();
            r.notify_when = c.notify_when;
            r
        })
        .collect()
}

/// Run the file-watcher loop. This future never returns normally — it
/// runs until `shutdown` is cancelled.
///
/// On startup and every `RELOAD_INTERVAL`, the watcher:
/// 1. Loads all enabled file watches from `store`.
/// 2. Starts/stops `notify` watchers to match the current set.
/// 3. Fires triggers through `dispatch` when events arrive, subject
///    to per-watch debounce.
pub async fn run_file_watcher(
    dispatch: TriggerDispatch,
    store: DomainHandle,
    shutdown: CancellationToken,
) {
    let (tx, mut rx) = mpsc::channel::<(String, Event)>(256);

    // Shared state: watch_id → (path, last_fired_at, debounce_ms)
    let mut active_watches: HashMap<String, WatchState> = HashMap::new();
    let mut watcher: Option<RecommendedWatcher> = None;

    loop {
        // Reload watches from storage.
        match file_watch::list_file_watches(&store).await {
            Ok(records) => {
                let enabled: Vec<_> = records.into_iter().filter(|r| r.enabled).collect();
                reconcile_watches(&mut watcher, &mut active_watches, &enabled, &tx);
            }
            Err(e) => {
                eprintln!("aivyx file-watcher: failed to load watches: {e}");
            }
        }

        // Process events until the next reload interval.
        let reload_deadline = tokio::time::Instant::now() + RELOAD_INTERVAL;

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep_until(reload_deadline) => break,
                Some((watch_id, _event)) = rx.recv() => {
                    if let Some(state) = active_watches.get_mut(&watch_id) {
                        if state.should_fire() {
                            state.mark_fired();
                            let dispatch = dispatch.clone();
                            let id = watch_id.clone();
                            let prompt = state.prompt.clone();
                            let wrap = state.wrap_mission;
                            let notify_targets = state.notify_targets.clone();
                            let notify_when = state.notify_when;
                            let store = store.clone();
                            tokio::spawn(async move {
                                dispatch
                                    .fire(
                                        TriggerSource::FileWatch,
                                        &id,
                                        &prompt,
                                        wrap,
                                        &notify_targets,
                                        notify_when,
                                    )
                                    .await;
                                // Update last_fired_at in storage.
                                update_last_fired(&store, &id).await;
                            });
                        }
                    }
                }
            }
        }
    }
}

struct WatchState {
    path: PathBuf,
    prompt: String,
    debounce_ms: u64,
    wrap_mission: bool,
    last_fired_ms: Option<u64>,
    /// Phase 72 — copied from `FileWatchRecord::notify_targets`
    /// at reconcile time; passed to `TriggerDispatch::fire` when
    /// the watcher fires. Empty = no notify.
    notify_targets: Vec<String>,
    /// Phase 72 — conditional dispatch gate copied from the
    /// record at reconcile time.
    notify_when: aivyx_config::NotifyWhen,
}

impl WatchState {
    fn should_fire(&self) -> bool {
        match self.last_fired_ms {
            Some(last) => {
                let now = now_millis();
                now.saturating_sub(last) >= self.debounce_ms
            }
            None => true,
        }
    }

    fn mark_fired(&mut self) {
        self.last_fired_ms = Some(now_millis());
    }
}

/// Reconcile the set of active `notify` watchers with the current
/// enabled file-watch records. Rebuilds the watcher if the watch set
/// has changed.
fn reconcile_watches(
    watcher: &mut Option<RecommendedWatcher>,
    active: &mut HashMap<String, WatchState>,
    records: &[FileWatchRecord],
    tx: &mpsc::Sender<(String, Event)>,
) {
    // Build desired state.
    let mut desired: HashMap<String, &FileWatchRecord> = HashMap::new();
    for r in records {
        desired.insert(r.watch_id.clone(), r);
    }

    // Check if reconciliation is needed.
    let current_ids: std::collections::HashSet<_> = active.keys().cloned().collect();
    let desired_ids: std::collections::HashSet<_> = desired.keys().cloned().collect();

    let paths_changed = active.iter().any(|(id, state)| {
        desired.get(id).is_some_and(|r| Path::new(&r.path) != state.path)
    });

    if current_ids == desired_ids && !paths_changed {
        return; // No change needed.
    }

    // Rebuild the watcher from scratch. This is simpler than incremental
    // add/remove and happens at most once per RELOAD_INTERVAL.
    let tx = tx.clone();

    // Build a path → watch_id lookup for the event handler.
    let mut path_to_id: HashMap<PathBuf, String> = HashMap::new();
    for r in records {
        let p = PathBuf::from(&r.path);
        // Canonicalize if possible, fall back to the raw path.
        let canonical = std::fs::canonicalize(&p).unwrap_or(p);
        path_to_id.insert(canonical, r.watch_id.clone());
    }
    let path_to_id = Arc::new(path_to_id);

    let lookup = Arc::clone(&path_to_id);
    let new_watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        match res {
            Ok(event) => {
                for path in &event.paths {
                    // Try exact match first, then check parent directories.
                    let canonical = std::fs::canonicalize(path)
                        .unwrap_or_else(|_| path.clone());

                    let watch_id = lookup.get(&canonical)
                        .or_else(|| {
                            // Check if any watched directory is an ancestor.
                            lookup.iter().find_map(|(watched, id)| {
                                if canonical.starts_with(watched) {
                                    Some(id)
                                } else {
                                    None
                                }
                            })
                        });

                    if let Some(id) = watch_id {
                        let _ = tx.try_send((id.clone(), event.clone()));
                        break; // One event per fire, not per path.
                    }
                }
            }
            Err(e) => {
                eprintln!("aivyx file-watcher: notify error: {e}");
            }
        }
    });

    match new_watcher {
        Ok(mut w) => {
            // Set up watches.
            let mut new_active = HashMap::new();
            for r in records {
                let p = PathBuf::from(&r.path);
                let mode = if p.is_dir() {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                };
                match w.watch(&p, mode) {
                    Ok(()) => {
                        new_active.insert(r.watch_id.clone(), WatchState {
                            path: p,
                            prompt: r.prompt.clone(),
                            debounce_ms: r.debounce_ms,
                            wrap_mission: r.wrap_mission,
                            last_fired_ms: r.last_fired_at,
                            notify_targets: r.notify_targets.clone(),
                            notify_when: r.notify_when,
                        });
                    }
                    Err(e) => {
                        eprintln!(
                            "aivyx file-watcher: failed to watch {:?} for {:?}: {e}",
                            r.path, r.watch_id,
                        );
                    }
                }
            }

            if let Err(e) = w.configure(Config::default()) {
                eprintln!("aivyx file-watcher: configure error: {e}");
            }

            *active = new_active;
            *watcher = Some(w);
        }
        Err(e) => {
            eprintln!("aivyx file-watcher: failed to create watcher: {e}");
        }
    }
}

async fn update_last_fired(store: &DomainHandle, watch_id: &str) {
    match file_watch::get_file_watch(store, watch_id).await {
        Ok(Some(mut record)) => {
            record.last_fired_at = Some(now_millis());
            if let Err(e) = file_watch::update_file_watch(store, &record).await {
                eprintln!(
                    "aivyx file-watcher: failed to update last_fired_at for {watch_id}: {e}"
                );
            }
        }
        Ok(None) => {} // Watch was deleted between fire and update.
        Err(e) => {
            eprintln!("aivyx file-watcher: storage read error for {watch_id}: {e}");
        }
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
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
    fn config_to_records_converts_correctly() {
        let configs = vec![aivyx_config::FileWatchConfig {
            name: "data-dir".into(),
            path: "/tmp/data".into(),
            role: "ops".into(),
            prompt: "new data arrived".into(),
            enabled: true,
            debounce_ms: Some(5000),
            wrap_mission: false,
            notify_target: None,
            notify_targets: Vec::new(),
            notify_when: aivyx_config::NotifyWhen::Always,
        }];
        let records = config_to_records(&configs);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].watch_id, "cfg-data-dir");
        assert_eq!(records[0].path, "/tmp/data");
        assert_eq!(records[0].debounce_ms, 5000);
    }

    #[test]
    fn config_to_records_uses_default_debounce() {
        let configs = vec![aivyx_config::FileWatchConfig {
            name: "logs".into(),
            path: "/var/log".into(),
            role: "default".into(),
            prompt: "check logs".into(),
            enabled: true,
            debounce_ms: None,
            wrap_mission: false,
            notify_target: None,
            notify_targets: Vec::new(),
            notify_when: aivyx_config::NotifyWhen::Always,
        }];
        let records = config_to_records(&configs);
        assert_eq!(records[0].debounce_ms, file_watch::DEFAULT_DEBOUNCE_MS);
    }

    #[test]
    fn disabled_config_produces_disabled_record() {
        let configs = vec![aivyx_config::FileWatchConfig {
            name: "off".into(),
            path: "/tmp".into(),
            role: "default".into(),
            prompt: "test".into(),
            enabled: false,
            debounce_ms: None,
            wrap_mission: false,
            notify_target: None,
            notify_targets: Vec::new(),
            notify_when: aivyx_config::NotifyWhen::Always,
        }];
        let records = config_to_records(&configs);
        assert!(!records[0].enabled);
    }

    #[test]
    fn config_to_records_propagates_wrap_mission() {
        let configs = vec![aivyx_config::FileWatchConfig {
            name: "wrapped".into(),
            path: "/tmp/data".into(),
            role: "default".into(),
            prompt: "process".into(),
            enabled: true,
            debounce_ms: None,
            wrap_mission: true,
            notify_target: None,
            notify_targets: Vec::new(),
            notify_when: aivyx_config::NotifyWhen::Always,
        }];
        let records = config_to_records(&configs);
        assert!(records[0].wrap_mission);
    }
}
