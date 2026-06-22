//! KitchenDB PostgREST RPC client.
//!
//! Chapter Brigade (BG.1). The kitchen toolkit never reimplements domain logic
//! — it calls the operator's KitchenDB stored procedures. Every call is a
//! `POST <base_url>/rpc/<function>` with the operator's params plus the tenant
//! key `p_organization_id`, authenticated with the PostgREST `apikey` header +
//! `Authorization: Bearer`. This module owns that one mechanic; the tools are
//! thin wrappers that name a function + map their input to params.

use reqwest::Client;
use serde_json::{Map, Value};
use thiserror::Error;

/// Why a KitchenDB RPC call failed. Each maps to a tool `Failed` outcome with
/// an operator-/LLM-readable message.
#[derive(Debug, Error)]
pub enum KitchenError {
    #[error("params for `{function}` must be a JSON object")]
    BadParams { function: String },
    #[error("KitchenDB request to `{function}` failed: {source}")]
    Http {
        function: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("KitchenDB `{function}` returned HTTP {status}: {body}")]
    Status {
        function: String,
        status: u16,
        body: String,
    },
    #[error("KitchenDB `{function}` response was not valid JSON: {reason}")]
    Parse { function: String, reason: String },
}

/// A connection to one KitchenDB (PostgREST) tenant.
#[derive(Clone)]
pub struct KitchenClient {
    http: Client,
    /// Base URL with any trailing `/` stripped; RPCs append `/rpc/<fn>`.
    base_url: String,
    api_key: String,
    organization_id: String,
}

impl KitchenClient {
    pub fn new(
        http: Client,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        organization_id: impl Into<String>,
    ) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Self {
            http,
            base_url,
            api_key: api_key.into(),
            organization_id: organization_id.into(),
        }
    }

    /// The tenant id this client scopes every call to.
    pub fn organization_id(&self) -> &str {
        &self.organization_id
    }

    /// Call a KitchenDB RPC. `params` must be a JSON object (or `Null`/absent
    /// → an empty object); `p_organization_id` is injected automatically and
    /// always wins over any caller-supplied value (the tenant is the client's,
    /// not the LLM's, to choose). Returns the parsed JSON response body.
    pub async fn call_rpc(&self, function: &str, params: Value) -> Result<Value, KitchenError> {
        let mut body: Map<String, Value> = match params {
            Value::Null => Map::new(),
            Value::Object(m) => m,
            _ => return Err(KitchenError::BadParams { function: function.to_string() }),
        };
        // The tenant key is the client's to set — never the caller's.
        body.insert(
            "p_organization_id".to_string(),
            Value::String(self.organization_id.clone()),
        );

        let url = format!("{}/rpc/{}", self.base_url, function);
        let response = self
            .http
            .post(&url)
            .header("apikey", &self.api_key)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "application/json")
            .json(&Value::Object(body))
            .send()
            .await
            .map_err(|source| KitchenError::Http { function: function.to_string(), source })?;

        let status = response.status();
        let text = response.text().await.map_err(|source| KitchenError::Http {
            function: function.to_string(),
            source,
        })?;
        if !status.is_success() {
            return Err(KitchenError::Status {
                function: function.to_string(),
                status: status.as_u16(),
                body: text,
            });
        }
        serde_json::from_str(&text).map_err(|e| KitchenError::Parse {
            function: function.to_string(),
            reason: e.to_string(),
        })
    }
}

#[cfg(test)]
pub(crate) mod mock {
    //! A minimal in-process PostgREST mock — captures the request line, the
    //! `apikey` header, and the JSON body; replies with a canned status + body.
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    /// What the mock captured from the single request it served.
    #[derive(Clone, Default, Debug)]
    pub struct Captured {
        pub path: String,
        pub apikey: String,
        pub authorization: String,
        pub body: String,
    }

    /// Spawn a one-shot mock; returns its base URL (no trailing slash) + the
    /// capture handle (populated after the request is served).
    pub async fn spawn(status: u16, body: &'static str) -> (String, Arc<Mutex<Captured>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let captured: Arc<Mutex<Captured>> = Arc::new(Mutex::new(Captured::default()));
        let cap = Arc::clone(&captured);
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let (read_half, mut write_half) = sock.split();
            let mut reader = BufReader::new(read_half);
            let mut request_line = String::new();
            reader.read_line(&mut request_line).await.unwrap();
            let path = request_line.split_whitespace().nth(1).unwrap_or("").to_string();
            let mut apikey = String::new();
            let mut authorization = String::new();
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 || line == "\r\n" {
                    break;
                }
                if let Some(idx) = line.find(':') {
                    let name = line[..idx].to_ascii_lowercase();
                    let value = line[idx + 1..].trim().to_string();
                    match name.as_str() {
                        "apikey" => apikey = value,
                        "authorization" => authorization = value,
                        "content-length" => content_length = value.parse().unwrap_or(0),
                        _ => {}
                    }
                }
            }
            let mut body_buf = vec![0u8; content_length];
            if content_length > 0 {
                let _ = reader.read_exact(&mut body_buf).await;
            }
            *cap.lock().unwrap() = Captured {
                path,
                apikey,
                authorization,
                body: String::from_utf8_lossy(&body_buf).to_string(),
            };
            let phrase = if (200..300).contains(&status) { "OK" } else { "Err" };
            let resp = format!(
                "HTTP/1.1 {status} {phrase}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body,
            );
            write_half.write_all(resp.as_bytes()).await.unwrap();
            write_half.flush().await.unwrap();
        });
        (format!("http://127.0.0.1:{port}"), captured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn call_rpc_posts_to_rpc_path_with_auth_and_tenant() {
        let (base, captured) = mock::spawn(200, r#"[{"sku":"TOM-01"}]"#).await;
        let client = KitchenClient::new(Client::new(), base, "MYKEY", "ORG-123");
        let out = client
            .call_rpc("get_inventory", json!({ "p_location": "walk-in" }))
            .await
            .expect("rpc ok");
        // Response parsed through.
        assert_eq!(out, json!([{"sku": "TOM-01"}]));
        // Request shape captured.
        let c = captured.lock().unwrap().clone();
        assert_eq!(c.path, "/rpc/get_inventory", "{}", c.path);
        assert_eq!(c.apikey, "MYKEY");
        assert_eq!(c.authorization, "Bearer MYKEY");
        let body: Value = serde_json::from_str(&c.body).unwrap();
        // The tenant key is injected; the caller's param survives.
        assert_eq!(body["p_organization_id"], "ORG-123");
        assert_eq!(body["p_location"], "walk-in");
    }

    #[tokio::test]
    async fn call_rpc_injected_tenant_overrides_caller_supplied() {
        let (base, captured) = mock::spawn(200, "[]").await;
        let client = KitchenClient::new(Client::new(), base, "K", "REAL-ORG");
        client
            .call_rpc("get_inventory", json!({ "p_organization_id": "SPOOFED" }))
            .await
            .expect("ok");
        let body: Value =
            serde_json::from_str(&captured.lock().unwrap().body).unwrap();
        assert_eq!(body["p_organization_id"], "REAL-ORG", "client tenant must win");
    }

    #[tokio::test]
    async fn call_rpc_null_params_sends_just_the_tenant() {
        let (base, captured) = mock::spawn(200, "[]").await;
        let client = KitchenClient::new(Client::new(), base, "K", "ORG");
        client.call_rpc("get_suppliers", Value::Null).await.expect("ok");
        let body: Value =
            serde_json::from_str(&captured.lock().unwrap().body).unwrap();
        assert_eq!(body, json!({ "p_organization_id": "ORG" }));
    }

    #[tokio::test]
    async fn call_rpc_non_object_params_is_rejected_without_a_request() {
        let client = KitchenClient::new(Client::new(), "http://127.0.0.1:9", "K", "ORG");
        let err = client.call_rpc("x", json!("a string")).await.unwrap_err();
        assert!(matches!(err, KitchenError::BadParams { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn call_rpc_surfaces_http_error_status_with_body() {
        let (base, _) = mock::spawn(404, r#"{"message":"no such function"}"#).await;
        let client = KitchenClient::new(Client::new(), base, "K", "ORG");
        let err = client.call_rpc("nope", json!({})).await.unwrap_err();
        match err {
            KitchenError::Status { status, body, .. } => {
                assert_eq!(status, 404);
                assert!(body.contains("no such function"), "{body}");
            }
            other => panic!("expected Status; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn call_rpc_surfaces_parse_error_on_non_json_body() {
        let (base, _) = mock::spawn(200, "not json").await;
        let client = KitchenClient::new(Client::new(), base, "K", "ORG");
        let err = client.call_rpc("x", json!({})).await.unwrap_err();
        assert!(matches!(err, KitchenError::Parse { .. }), "{err:?}");
    }

    #[test]
    fn new_trims_trailing_slash_from_base_url() {
        let c = KitchenClient::new(Client::new(), "https://k/rest/v1/", "K", "ORG");
        assert_eq!(c.base_url, "https://k/rest/v1");
    }
}
