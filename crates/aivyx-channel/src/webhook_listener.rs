//! Webhook HTTP listener — Phase 27 Task 3.
//!
//! A minimal `hyper` HTTP/1.1 server bound to `127.0.0.1` that accepts
//! `POST /trigger/<webhook_id>` requests and fires the associated
//! webhook's prompt through the shared `TriggerDispatch`. Non-matching
//! paths and methods return 404/405. The listener runs as a background
//! task inside the daemon, alongside the cron scheduler.
//!
//! The server is localhost-only per PRODUCT.md P6 (local execution,
//! privacy): "A webhook-triggered run is delivered by a daemon thread
//! already running under the operator's OS user."

use std::sync::Arc;

use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use http_body_util::Full;
use tokio::net::TcpListener;

use aivyx_core::CancellationToken;
use aivyx_storage::DomainHandle;

use crate::trigger::{TriggerDispatch, TriggerSource};
use crate::webhook;

/// Default webhook listener port.
pub const DEFAULT_WEBHOOK_PORT: u16 = 7842;

/// Run the webhook HTTP listener. This future never returns normally —
/// it runs until `shutdown` is cancelled.
pub async fn run_webhook_listener(
    dispatch: TriggerDispatch,
    store: DomainHandle,
    port: u16,
    shutdown: CancellationToken,
) -> Result<(), String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("webhook listener: failed to bind {addr}: {e}"))?;

    eprintln!("aivyx webhook: listening on http://{addr}");

    let store = Arc::new(store);

    loop {
        let (stream, _remote) = tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok(conn) => conn,
                    Err(e) => {
                        eprintln!("aivyx webhook: accept error: {e}");
                        continue;
                    }
                }
            }
            _ = shutdown.cancelled() => return Ok(()),
        };

        let dispatch = dispatch.clone();
        let store = Arc::clone(&store);
        let conn_shutdown = shutdown.clone();

        tokio::spawn(async move {
            let svc = service_fn(move |req| {
                let dispatch = dispatch.clone();
                let store = Arc::clone(&store);
                async move { handle_request(req, &dispatch, &store).await }
            });

            let io = TokioIo::new(stream);
            let conn = http1::Builder::new().serve_connection(io, svc);
            tokio::select! {
                result = conn => {
                    if let Err(e) = result {
                        eprintln!("aivyx webhook: connection error: {e}");
                    }
                }
                _ = conn_shutdown.cancelled() => {}
            }
        });
    }
}

/// Handle a single HTTP request. Only `POST /trigger/<id>` is valid.
async fn handle_request(
    req: Request<Incoming>,
    dispatch: &TriggerDispatch,
    store: &DomainHandle,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let path = req.uri().path().to_owned();
    let method = req.method().clone();

    // Route: POST /trigger/<webhook_id>
    if let Some(webhook_id) = path.strip_prefix("/trigger/") {
        if webhook_id.is_empty() {
            return Ok(json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error":"missing webhook id"}"#,
            ));
        }

        if method != hyper::Method::POST {
            return Ok(json_response(
                StatusCode::METHOD_NOT_ALLOWED,
                r#"{"error":"use POST"}"#,
            ));
        }

        return Ok(fire_webhook(dispatch, store, webhook_id).await);
    }

    // Health check endpoint
    if path == "/health" && method == hyper::Method::GET {
        return Ok(json_response(StatusCode::OK, r#"{"status":"ok"}"#));
    }

    Ok(json_response(
        StatusCode::NOT_FOUND,
        r#"{"error":"not found"}"#,
    ))
}

/// Look up and fire a webhook by ID.
async fn fire_webhook(
    dispatch: &TriggerDispatch,
    store: &DomainHandle,
    webhook_id: &str,
) -> Response<Full<Bytes>> {
    let record = match webhook::get_webhook(store, webhook_id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return json_response(
                StatusCode::NOT_FOUND,
                r#"{"error":"webhook not found"}"#,
            );
        }
        Err(e) => {
            eprintln!("aivyx webhook: storage error looking up {webhook_id:?}: {e}");
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"error":"internal error"}"#,
            );
        }
    };

    if !record.enabled {
        return json_response(
            StatusCode::CONFLICT,
            r#"{"error":"webhook is disabled"}"#,
        );
    }

    // Fire asynchronously — the HTTP response returns immediately
    // with "accepted", and the turn runs in the background.
    let dispatch = dispatch.clone();
    let id = record.webhook_id.clone();
    let prompt = record.prompt.clone();
    let store_clone = store.clone();
    tokio::spawn(async move {
        dispatch.fire(TriggerSource::Webhook, &id, &prompt).await;

        // Update last_fired_at.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let mut updated = record;
        updated.last_fired_at = Some(now_ms);
        if let Err(e) = webhook::update_webhook(&store_clone, &updated).await {
            eprintln!("aivyx webhook: failed to update last_fired_at for {}: {e}", updated.webhook_id);
        }
    });

    json_response(StatusCode::ACCEPTED, r#"{"status":"accepted"}"#)
}

fn json_response(status: StatusCode, body: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body.to_owned())))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_response_sets_content_type() {
        let resp = json_response(StatusCode::OK, r#"{"ok":true}"#);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn json_response_not_found() {
        let resp = json_response(StatusCode::NOT_FOUND, r#"{"error":"nope"}"#);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
    }
}
