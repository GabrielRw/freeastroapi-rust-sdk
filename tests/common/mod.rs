//! Shared helpers for the integration tests.
#![allow(dead_code)]

use freeastroapi::Client;
use serde_json::Value;
use wiremock::{Match, MockServer, Request, ResponseTemplate};

pub const API_KEY: &str = "test-key";

/// A mock server and a client pointed at it (no retry delays).
pub async fn setup() -> (MockServer, Client) {
    let server = MockServer::start().await;
    let client = client_for(&server, 0);
    (server, client)
}

pub fn client_for(server: &MockServer, max_retries: u32) -> Client {
    Client::builder()
        .api_key(API_KEY)
        .base_url(server.uri())
        .max_retries(max_retries)
        .retry_base_delay(std::time::Duration::from_millis(1))
        .build()
        .expect("client builds")
}

pub fn json_response(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.as_bytes().to_vec(), "application/json")
}

pub fn raw_response(status: u16, content_type: &str, body: &[u8]) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.to_vec(), content_type)
}

/// Matches a JSON request body equal to `expected`, comparing numbers by value (`1` == `1.0`).
pub fn loose_json(expected: &str) -> LooseJson {
    LooseJson(serde_json::from_str(expected).expect("expected body is JSON"))
}

pub struct LooseJson(Value);

impl Match for LooseJson {
    fn matches(&self, request: &Request) -> bool {
        match serde_json::from_slice::<Value>(&request.body) {
            Ok(actual) => loose_eq(&self.0, &actual),
            Err(_) => false,
        }
    }
}

pub fn loose_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| loose_eq(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| loose_eq(v, w)))
        }
        _ => a == b,
    }
}

/// Contract check for one operation against the Prism mock at `PRISM_URL`.
///
/// 1. The SDK call is captured by a local recorder, so the exact request bytes are known.
/// 2. That request is replayed to Prism, which validates it against the spec: the response must
///    be 2xx with no request violations in `sl-violations` (Prism answers 500 "Cannot find
///    serializer" for schemaless image responses after validating the request; that is accepted).
/// 3. The SDK call is repeated against Prism and its response must decode into the method's
///    return type, unless Prism reported that its own mock body breaks the response schema.
pub async fn contract<T, F, Fut>(operation_id: &str, call: F)
where
    F: Fn(Client) -> Fut,
    Fut: std::future::Future<Output = freeastroapi::Result<T>>,
{
    let prism = std::env::var("PRISM_URL")
        .expect("set PRISM_URL or run: python3 scripts/with_prism.py -- cargo test --test contract -- --ignored");

    let recorder = MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(json_response(200, "null"))
        .mount(&recorder)
        .await;
    let _ = call(client_for(&recorder, 0)).await;
    let sent = recorder
        .received_requests()
        .await
        .unwrap()
        .pop()
        .expect("the SDK sent a request");

    let url = match sent.url.query() {
        Some(query) => format!("{prism}{}?{query}", sent.url.path()),
        None => format!("{prism}{}", sent.url.path()),
    };
    let method =
        freeastroapi::reqwest::Method::from_bytes(sent.method.as_str().as_bytes()).unwrap();
    let mut replay = freeastroapi::reqwest::Client::new()
        .request(method, url)
        .body(sent.body.clone());
    for (name, value) in sent.headers.iter() {
        if name != "host" && name != "content-length" {
            replay = replay.header(name.as_str(), value.as_bytes());
        }
    }
    let response = replay.send().await.expect("Prism is reachable");
    let status = response.status().as_u16();
    let violations: Vec<Value> = response
        .headers()
        .get("sl-violations")
        .and_then(|v| serde_json::from_slice(v.as_bytes()).ok())
        .unwrap_or_default();
    let body = response.bytes().await.unwrap();
    let location = |v: &Value, part: &str| v["location"][0].as_str() == Some(part);
    let request_violations: Vec<&Value> = violations
        .iter()
        .filter(|v| location(v, "request"))
        .collect();
    if status == 500 && String::from_utf8_lossy(&body).contains("Cannot find serializer") {
        eprintln!(
            "{operation_id}: request valid; Prism cannot mock this schemaless body (accepted)"
        );
        return;
    }
    assert!(
        status < 300 && request_violations.is_empty(),
        "{operation_id}: {} {} -> {status}\nrequest violations: {request_violations:#?}\nbody: {}",
        sent.method,
        sent.url,
        String::from_utf8_lossy(&body)
    );

    let prism_client = Client::builder()
        .api_key("contract-test-key")
        .base_url(&prism)
        .max_retries(0)
        .build()
        .unwrap();
    match call(prism_client).await {
        Ok(_) => {}
        Err(freeastroapi::Error::Decode { source, .. })
            if violations.iter().any(|v| location(v, "response")) =>
        {
            eprintln!("{operation_id}: Prism's mock body breaks the schema ({source}); accepted");
        }
        Err(error) => panic!("{operation_id}: response does not decode: {error:?}"),
    }
}
