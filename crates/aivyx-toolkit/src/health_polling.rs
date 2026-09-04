//! Background polling loop for `health.check`.
//!
//! Phase 125 Task 5. Spawned once at tool-process startup
//! (Task 6 wires this into `main.rs`). The loop:
//!
//! 1. Asks the store for due watchers at the current time.
//! 2. Probes each due watcher with a single GET against the
//!    configured URL (10-second timeout per probe).
//! 3. Records each outcome back into the store, which detects
//!    state transitions and updates the ring buffer.
//! 4. Asks the store for the time until the next watcher is
//!    due; sleeps that long (clamped to `MIN_SLEEP` /
//!    `MAX_SLEEP`).
//! 5. Repeats forever until the tokio runtime is shut down.
//!
//! Clean shutdown happens implicitly when the tool process's
//! `main()` returns and tokio aborts the spawned task. Phase
//! 125 doesn't ship explicit cancellation; the daemon's
//! `ToolShutdown` frame triggers the harness to return from
//! `run_multi_tool_subprocess`, which returns from `main`,
//! which aborts this task. Honest scope: an in-flight HTTP
//! probe gets aborted mid-call, which is fine — the store
//! state hasn't been updated yet, so we just lose the
//! attempt.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use reqwest::Client;

use crate::health_store::{HealthStore, ProbeOutcome, Watcher};

/// Minimum sleep between batches. Stops a runaway loop from
/// burning CPU if the store reports zero-wait repeatedly
/// (which can happen if watchers are due in less than 1s due
/// to interval/clock granularity).
const MIN_SLEEP: Duration = Duration::from_secs(1);

/// Maximum sleep between batches. Provides a heartbeat in
/// case the store changes (operator adds a watcher) and the
/// loop hasn't otherwise been woken.
const MAX_SLEEP: Duration = Duration::from_secs(30);

/// Per-probe HTTP timeout. Long enough for slow services;
/// short enough that a hung watcher doesn't block the whole
/// loop for minutes.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Run the polling loop forever. The function never returns;
/// the tokio runtime aborts the task on shutdown.
pub async fn run_polling_loop(
    store: Arc<HealthStore>,
    http: Client,
    notify_tx: tokio::sync::mpsc::UnboundedSender<aivyx_tool::wire::ToolToDaemon>,
    default_notify_target: Option<String>,
) -> ! {
    loop {
        run_polling_tick(&store, &http, Utc::now(), &notify_tx, &default_notify_target).await;
        let wait = match store.next_check_in(Utc::now()).await {
            Some(d) => d.clamp(MIN_SLEEP, MAX_SLEEP),
            None => MAX_SLEEP,
        };
        tokio::time::sleep(wait).await;
    }
}

/// One pass of the loop body. Public so tests can exercise
/// the tick without spinning the infinite loop.
pub async fn run_polling_tick(
    store: &HealthStore,
    http: &Client,
    now: chrono::DateTime<Utc>,
    notify_tx: &tokio::sync::mpsc::UnboundedSender<aivyx_tool::wire::ToolToDaemon>,
    default_notify_target: &Option<String>,
) {
    let due = store.due_watchers(now).await;
    for watcher in due {
        let outcome = probe(http, &watcher).await;
        // Record-check failures are silently dropped — the
        // store's `Io` error means the file system is in a
        // bad state and the operator will see it surface at
        // the next tool invocation anyway. Logging here would
        // need a structured surface we don't have inside the
        // tool process.
        if let Ok(Some(transition)) = store.record_check(&watcher.name, outcome, Utc::now()).await {
            if let Some(target) = default_notify_target {
                let direction = if transition.to_ok { "RECOVERED" } else { "DOWN" };
                let status = transition
                    .status_code
                    .map(|c| format!(" (status {c})"))
                    .unwrap_or_default();
                let message = format!(
                    "Health watcher '{}' went {direction}{status}",
                    transition.watcher_name
                );
                let _ = notify_tx.send(aivyx_tool::wire::ToolToDaemon::DispatchNotification {
                    target: target.clone(),
                    message,
                    subject: Some("Health alert".to_string()),
                });
            }
            // default_notify_target unset: skip silently, per Global
            // Constraints — no notify_tx.send attempted at all.
        }
    }
}

/// Issue one HTTP GET against a watcher's URL and translate
/// the response into a [`ProbeOutcome`]. `ok` is true iff
/// the response status equals the watcher's
/// `expect_status`.
///
/// Network errors (DNS failure, connect timeout, TLS error,
/// etc) surface as `ProbeOutcome { status_code: None, ok:
/// false }`. The probe never panics.
pub async fn probe(http: &Client, watcher: &Watcher) -> ProbeOutcome {
    let response = match http
        .get(&watcher.url)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            return ProbeOutcome {
                status_code: None,
                ok: false,
            };
        }
    };
    let status_u16 = response.status().as_u16();
    ProbeOutcome {
        status_code: Some(status_u16),
        ok: status_u16 == watcher.expect_status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health_store::HealthStore;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    fn scratch_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let tmp = std::env::var("TMPDIR")
            .or_else(|_| std::env::var("TEMP"))
            .unwrap_or_else(|_| "/tmp".to_string());
        let dir = PathBuf::from(tmp).join(format!(
            "aivyx-toolkit-polling-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Spawn a tiny HTTP server that returns the canned
    /// status for every request, then drops. Returns the
    /// URL.
    async fn spawn_mock_server(canned_status: u16) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            // Serve forever (until aborted by runtime
            // shutdown) so multiple sequential requests work.
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                tokio::spawn(async move {
                    let (read_half, mut write_half) = sock.split();
                    let mut reader = BufReader::new(read_half);
                    // Read the request line + headers.
                    loop {
                        let mut line = String::new();
                        let n = match reader.read_line(&mut line).await {
                            Ok(n) => n,
                            Err(_) => return,
                        };
                        if n == 0 || line == "\r\n" {
                            break;
                        }
                    }
                    let phrase = match canned_status {
                        200 => "OK",
                        404 => "Not Found",
                        503 => "Service Unavailable",
                        _ => "Status",
                    };
                    let body = format!("status={canned_status}");
                    let response = format!(
                        "HTTP/1.1 {canned_status} {phrase}\r\n\
                         Content-Type: text/plain\r\n\
                         Content-Length: {}\r\n\
                         Connection: close\r\n\
                         \r\n\
                         {}",
                        body.len(),
                        body,
                    );
                    let _ = write_half.write_all(response.as_bytes()).await;
                    let _ = write_half.flush().await;
                });
            }
        });
        format!("http://127.0.0.1:{port}/")
    }

    #[tokio::test]
    async fn probe_returns_ok_on_matching_status() {
        let url = spawn_mock_server(200).await;
        let watcher = Watcher {
            name: "x".to_string(),
            url: url.clone(),
            interval_secs: 60,
            expect_status: 200,
        };
        let outcome = probe(&Client::new(), &watcher).await;
        assert_eq!(outcome.status_code, Some(200));
        assert!(outcome.ok);
    }

    #[tokio::test]
    async fn probe_returns_not_ok_on_status_mismatch() {
        let url = spawn_mock_server(503).await;
        let watcher = Watcher {
            name: "x".to_string(),
            url,
            interval_secs: 60,
            expect_status: 200,
        };
        let outcome = probe(&Client::new(), &watcher).await;
        assert_eq!(outcome.status_code, Some(503));
        assert!(!outcome.ok);
    }

    #[tokio::test]
    async fn probe_returns_not_ok_on_network_failure() {
        // Point at an unbound port.
        let watcher = Watcher {
            name: "x".to_string(),
            url: "http://127.0.0.1:1/".to_string(),
            interval_secs: 60,
            expect_status: 200,
        };
        let outcome = probe(&Client::new(), &watcher).await;
        assert_eq!(outcome.status_code, None);
        assert!(!outcome.ok);
    }

    #[tokio::test]
    async fn polling_tick_updates_store_with_probe_outcome() {
        let dir = scratch_dir();
        let path = dir.join("health.json");
        let url = spawn_mock_server(200).await;
        let store = Arc::new(HealthStore::open(path).await.unwrap());
        store
            .add_watcher("x".to_string(), url, 60, 200)
            .await
            .unwrap();
        let http = Client::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        run_polling_tick(&store, &http, Utc::now(), &tx, &None).await;
        let watchers = store.list_watchers().await;
        assert_eq!(watchers.len(), 1);
        assert!(watchers[0].1.last_check_at.is_some());
        assert_eq!(watchers[0].1.last_status_code, Some(200));
        assert!(watchers[0].1.last_ok);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn polling_tick_records_transition_on_state_flip() {
        let dir = scratch_dir();
        let path = dir.join("health.json");
        let store = Arc::new(HealthStore::open(path).await.unwrap());
        // First tick — server returns 200 (ok).
        let ok_url = spawn_mock_server(200).await;
        store
            .add_watcher("x".to_string(), ok_url, 60, 200)
            .await
            .unwrap();
        let http = Client::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let t1 = Utc::now();
        run_polling_tick(&store, &http, t1, &tx, &None).await;

        // Second tick — now we replace the watcher's URL with
        // a mock that returns 503. We can't update the URL
        // through the store API (no remove tool yet); for
        // this test, build a fresh store at the same path,
        // then rewrite the watchers list manually before
        // re-loading.
        let down_url = spawn_mock_server(503).await;
        let watchers_path = dir.join("health.json");
        let body = std::fs::read_to_string(&watchers_path).unwrap();
        // Replace the URL in the JSON.
        let body = body.replace(&format!("\"url\": \"{}\"", url_from(&store).await), &format!("\"url\": \"{}\"", down_url));
        std::fs::write(&watchers_path, body).unwrap();
        let store = Arc::new(HealthStore::open(watchers_path).await.unwrap());
        // Advance time so the watcher is due again.
        let t2 = t1 + chrono::Duration::seconds(120);
        run_polling_tick(&store, &http, t2, &tx, &None).await;

        let transitions = store
            .recent_transitions_within(t2, Duration::from_secs(3600))
            .await;
        assert_eq!(transitions.len(), 1, "expected exactly one state flip");
        assert!(transitions[0].from_ok, "was ok");
        assert!(!transitions[0].to_ok, "now down");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Helper for the URL-swap test above. Returns the URL of
    /// the first watcher in the store.
    async fn url_from(store: &HealthStore) -> String {
        store.list_watchers().await[0].0.url.clone()
    }

    #[tokio::test]
    async fn tick_sends_dispatch_notification_on_down_transition() {
        let dir = scratch_dir();
        let path = dir.join("health.json");
        let store = Arc::new(HealthStore::open(path).await.unwrap());
        let ok_url = spawn_mock_server(200).await;
        store.add_watcher("x".to_string(), ok_url, 60, 200).await.unwrap();
        let http = Client::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // First tick: establishes the "up" baseline, no transition yet.
        let t1 = Utc::now();
        run_polling_tick(&store, &http, t1, &tx, &Some("phone".to_string())).await;
        assert!(rx.try_recv().is_err(), "no transition on the first-ever poll");

        // Rewrite the watcher's URL to a failing mock (same technique the
        // existing polling_tick_records_transition_on_state_flip test
        // already uses — no direct "update watcher URL" store API exists).
        let down_url = spawn_mock_server(503).await;
        let watchers_path = dir.join("health.json");
        let body = std::fs::read_to_string(&watchers_path).unwrap();
        let body = body.replace(&format!("\"url\": \"{}\"", url_from(&store).await), &format!("\"url\": \"{down_url}\""));
        std::fs::write(&watchers_path, body).unwrap();
        let store = Arc::new(HealthStore::open(watchers_path).await.unwrap());

        // Advance time so the watcher is due again (same technique as
        // polling_tick_records_transition_on_state_flip — record_check
        // stamps the real wall-clock time, so due_watchers must be given
        // a `now` far enough past that real timestamp).
        let t2 = t1 + chrono::Duration::seconds(120);
        run_polling_tick(&store, &http, t2, &tx, &Some("phone".to_string())).await;
        let sent = rx.try_recv().expect("expected a DispatchNotification on the down transition");
        match sent {
            aivyx_tool::wire::ToolToDaemon::DispatchNotification { target, message, .. } => {
                assert_eq!(target, "phone");
                assert!(message.contains("DOWN"), "message was: {message}");
            }
            other => panic!("expected DispatchNotification, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn tick_sends_nothing_when_no_target_configured() {
        // Deliberately drives a *real* transition (not just a first-ever
        // poll, which record_check never treats as a transition anyway)
        // so this test genuinely exercises the `default_notify_target ==
        // None` skip branch inside the dispatch code, rather than passing
        // trivially because no transition ever fired at all.
        let dir = scratch_dir();
        let path = dir.join("health.json");
        let store = Arc::new(HealthStore::open(path).await.unwrap());
        let ok_url = spawn_mock_server(200).await;
        store.add_watcher("x".to_string(), ok_url, 60, 200).await.unwrap();
        let http = Client::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // First tick: establishes the "up" baseline, no transition yet.
        let t1 = Utc::now();
        run_polling_tick(&store, &http, t1, &tx, &None).await;
        assert!(rx.try_recv().is_err(), "no transition on the first-ever poll");

        // Flip the watcher to failing, same URL-swap technique as the
        // sibling test above.
        let down_url = spawn_mock_server(503).await;
        let watchers_path = dir.join("health.json");
        let body = std::fs::read_to_string(&watchers_path).unwrap();
        let body = body.replace(&format!("\"url\": \"{}\"", url_from(&store).await), &format!("\"url\": \"{down_url}\""));
        std::fs::write(&watchers_path, body).unwrap();
        let store = Arc::new(HealthStore::open(watchers_path).await.unwrap());

        let t2 = t1 + chrono::Duration::seconds(120);
        run_polling_tick(&store, &http, t2, &tx, &None).await;
        let transitions = store
            .recent_transitions_within(t2, Duration::from_secs(3600))
            .await;
        assert_eq!(transitions.len(), 1, "sanity check: the transition really did fire");
        assert!(
            rx.try_recv().is_err(),
            "no target configured means no send at all, even on a real transition"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
