use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, StatusCode, Url};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::body::{Binary, Content};
use crate::error::{ApiError, Error, Result};

/// The production API.
pub const DEFAULT_BASE_URL: &str = "https://api.freeastroapi.com";
/// Environment variable read for the API key when the builder is not given one.
pub const API_KEY_ENV: &str = "FREEASTRO_API_KEY";
/// Default per-attempt timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default number of retries after the first attempt.
pub const DEFAULT_MAX_RETRIES: u32 = 2;

const USER_AGENT: &str = concat!("freeastroapi-rust/", env!("CARGO_PKG_VERSION"));
const RETRY_STATUSES: [u16; 6] = [408, 429, 500, 502, 503, 504];
const MAX_ERROR_BODY: usize = 4096;

/// FreeAstroAPI client.
///
/// Cheap to clone (the connection pool is shared). Operations are grouped by API path:
/// `client.vedic().panchang(&body)` calls `POST /api/v2/vedic/panchang`. Where an operation
/// exists in several API versions the plain name calls the latest one and older ones carry a
/// suffix, e.g. `client.vedic().panchang_v1(&body)`.
///
/// ```no_run
/// # async fn run() -> freeastroapi::Result<()> {
/// let client = freeastroapi::Client::builder()
///     .api_key("your-api-key")
///     .timeout(std::time::Duration::from_secs(20))
///     .max_retries(3)
///     .build()?;
/// # let _ = client;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
    options: RequestOptions,
}

struct Inner {
    http: reqwest::Client,
    base_url: String,
    api_key: HeaderValue,
    headers: HeaderMap,
    timeout: Duration,
    max_retries: u32,
    retry_base_delay: Duration,
    max_retry_delay: Duration,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.inner.base_url)
            .field("api_key", &"<redacted>")
            .field("timeout", &self.inner.timeout)
            .field("max_retries", &self.inner.max_retries)
            .field("options", &self.options)
            .finish()
    }
}

impl Client {
    /// Start configuring a client.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// A client with default settings and the given API key.
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        Self::builder().api_key(api_key).build()
    }

    /// A client with default settings and the API key from `FREEASTRO_API_KEY`.
    pub fn from_env() -> Result<Self> {
        Self::builder().build()
    }

    /// The base URL requests are sent to.
    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    /// A client that applies `options` to every request it sends (timeout, retries,
    /// idempotency key, extra headers). It shares the connection pool with `self`.
    ///
    /// ```no_run
    /// # async fn run(client: freeastroapi::Client) -> freeastroapi::Result<()> {
    /// use freeastroapi::RequestOptions;
    /// use std::time::Duration;
    ///
    /// let slow = client.with_options(RequestOptions::new().timeout(Duration::from_secs(120)));
    /// let credits = slow.natal().report_credits().await?;
    /// # let _ = credits;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_options(&self, options: RequestOptions) -> Client {
        Client {
            inner: Arc::clone(&self.inner),
            options: self.options.clone().merge(options),
        }
    }

    /// Call any endpoint and decode the JSON response. Prefer the typed methods; this is an
    /// escape hatch for operations added to the API after this SDK version.
    pub async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&serde_json::Value>,
    ) -> Result<T> {
        let mut spec = RequestSpec::new(method, path);
        spec.query = query
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        if let Some(body) = body {
            spec = spec.json(body)?;
        }
        self.send_json(spec).await
    }

    // Which of these helpers the generated code uses depends on the spec.
    #[allow(dead_code)]
    pub(crate) async fn send_json<T: DeserializeOwned>(&self, spec: RequestSpec) -> Result<T> {
        let response = self.execute(spec).await?;
        decode(&response.body)
    }

    // Which of these helpers the generated code uses depends on the spec.
    #[allow(dead_code)]
    pub(crate) async fn send_text(&self, spec: RequestSpec) -> Result<String> {
        let response = self.execute(spec).await?;
        Ok(String::from_utf8_lossy(&response.body).into_owned())
    }

    // Which of these helpers the generated code uses depends on the spec.
    #[allow(dead_code)]
    pub(crate) async fn send_binary(&self, spec: RequestSpec) -> Result<Binary> {
        let response = self.execute(spec).await?;
        Ok(Binary {
            content_type: response.content_type,
            bytes: response.body,
        })
    }

    // Which of these helpers the generated code uses depends on the spec.
    #[allow(dead_code)]
    pub(crate) async fn send_content<T: DeserializeOwned>(
        &self,
        spec: RequestSpec,
    ) -> Result<Content<T>> {
        let response = self.execute(spec).await?;
        if response.content_type.as_deref().is_none_or(is_json) {
            decode(&response.body).map(Content::Json)
        } else {
            Ok(Content::Binary(Binary {
                content_type: response.content_type,
                bytes: response.body,
            }))
        }
    }

    async fn execute(&self, spec: RequestSpec) -> Result<RawResponse> {
        let inner = &*self.inner;
        let options = &self.options;
        let mut url = Url::parse(&format!("{}{}", inner.base_url, spec.path))
            .map_err(|e| Error::Config(format!("invalid request URL: {e}")))?;
        if !spec.query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in &spec.query {
                pairs.append_pair(key, value);
            }
        }

        let mut headers = inner.headers.clone();
        for (name, value) in &options.headers {
            headers.insert(header_name(name)?, header_value(value)?);
        }
        headers.insert("x-api-key", inner.api_key.clone());
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static(spec.accept.unwrap_or("application/json")),
        );
        if spec.body.is_some() {
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
        }
        if let Some(key) = &options.idempotency_key {
            headers.insert("idempotency-key", header_value(key)?);
        }

        let timeout = options.timeout.unwrap_or(inner.timeout);
        let max_retries = options.max_retries.unwrap_or(inner.max_retries);
        let replay_safe = spec.replay_safe
            || options.idempotency_key.is_some()
            || matches!(spec.method, Method::GET | Method::HEAD | Method::OPTIONS);

        let mut attempt = 0u32;
        loop {
            let mut request = inner
                .http
                .request(spec.method.clone(), url.clone())
                .headers(headers.clone())
                .timeout(timeout);
            if let Some(body) = &spec.body {
                request = request.body(body.clone());
            }

            let retry_in = match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let response_headers = response.headers().clone();
                    let body = response.bytes().await?;
                    if status.is_success() {
                        return Ok(RawResponse {
                            content_type: response_headers
                                .get(header::CONTENT_TYPE)
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_owned),
                            body,
                        });
                    }
                    let retry_after = retry_after(&response_headers);
                    let retryable = RETRY_STATUSES.contains(&status.as_u16())
                        && (replay_safe || status == StatusCode::TOO_MANY_REQUESTS);
                    let delay = retry_after.unwrap_or_else(|| self.backoff(attempt));
                    if retryable && attempt < max_retries && delay <= inner.max_retry_delay {
                        delay
                    } else {
                        let request_id = response_headers
                            .get("x-request-id")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned);
                        let error = ApiError::new(
                            status.as_u16(),
                            status.canonical_reason(),
                            request_id,
                            &body,
                        );
                        return Err(Error::from_response(error, retry_after));
                    }
                }
                Err(error) => {
                    let transient = error.is_timeout() || error.is_connect() || error.is_request();
                    if transient && replay_safe && attempt < max_retries {
                        self.backoff(attempt)
                    } else {
                        return Err(Error::Http(error));
                    }
                }
            };
            attempt += 1;
            tokio::time::sleep(retry_in).await;
        }
    }

    /// Exponential backoff with jitter: `base * 2^attempt`, scaled by a random factor in
    /// [0.5, 1.0) and capped at the maximum retry delay.
    fn backoff(&self, attempt: u32) -> Duration {
        let base = self
            .inner
            .retry_base_delay
            .saturating_mul(1u32 << attempt.min(16));
        let capped = base.min(self.inner.max_retry_delay);
        capped.mul_f64(0.5 + jitter() / 2.0)
    }
}

fn jitter() -> f64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    (hasher.finish() >> 11) as f64 / (1u64 << 53) as f64
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(header::RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<f64>() {
        return (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds));
    }
    let at = httpdate::parse_http_date(value).ok()?;
    Some(at.duration_since(SystemTime::now()).unwrap_or_default())
}

fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T> {
    let body = if body.iter().all(u8::is_ascii_whitespace) {
        b"null".as_slice()
    } else {
        body
    };
    serde_json::from_slice(body).map_err(|source| Error::Decode {
        source,
        body: truncate(body),
    })
}

fn truncate(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(&body[..body.len().min(MAX_ERROR_BODY)]);
    text.into_owned()
}

#[allow(dead_code)]
fn is_json(content_type: &str) -> bool {
    let base = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    base == "application/json" || base.ends_with("+json")
}

fn header_name(name: &str) -> Result<HeaderName> {
    HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| Error::Config(format!("invalid header name {name:?}")))
}

fn header_value(value: &str) -> Result<HeaderValue> {
    HeaderValue::from_str(value).map_err(|_| Error::Config("invalid header value".to_owned()))
}

struct RawResponse {
    content_type: Option<String>,
    body: Bytes,
}

/// Configures a [`Client`].
#[derive(Debug, Clone)]
pub struct ClientBuilder {
    api_key: Option<String>,
    base_url: Option<String>,
    timeout: Duration,
    connect_timeout: Option<Duration>,
    max_retries: u32,
    retry_base_delay: Duration,
    max_retry_delay: Duration,
    headers: Vec<(String, String)>,
    user_agent: Option<String>,
    http_client: Option<reqwest::Client>,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        ClientBuilder {
            api_key: None,
            base_url: None,
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: Some(Duration::from_secs(10)),
            max_retries: DEFAULT_MAX_RETRIES,
            retry_base_delay: Duration::from_millis(500),
            max_retry_delay: Duration::from_secs(60),
            headers: Vec::new(),
            user_agent: None,
            http_client: None,
        }
    }
}

impl ClientBuilder {
    /// The API key sent as `x-api-key`. Defaults to the `FREEASTRO_API_KEY` environment variable.
    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// The API base URL. Defaults to [`DEFAULT_BASE_URL`].
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Timeout for each attempt, from sending the request to reading the whole body.
    /// Defaults to [`DEFAULT_TIMEOUT`].
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Timeout for establishing a connection (default 10 s). Ignored with [`Self::http_client`].
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// How many times a failed request is retried (default [`DEFAULT_MAX_RETRIES`]; 0 disables).
    ///
    /// 429 responses are always retried. Connection errors, timeouts and 408/500/502/503/504
    /// responses are retried for operations that are safe to replay: every read-only
    /// computation, and the others (chat, AI generation) when an idempotency key is set with
    /// [`RequestOptions::idempotency_key`]. A `Retry-After` header is honoured; otherwise the
    /// delay grows exponentially with jitter.
    pub fn max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// The first backoff delay, doubled on each retry (default 500 ms).
    pub fn retry_base_delay(mut self, delay: Duration) -> Self {
        self.retry_base_delay = delay;
        self
    }

    /// The longest the client waits between attempts (default 60 s). A `Retry-After` longer
    /// than this is not waited for: the error is returned with its `retry_after` instead.
    pub fn max_retry_delay(mut self, delay: Duration) -> Self {
        self.max_retry_delay = delay;
        self
    }

    /// A header sent with every request.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Text appended to the `User-Agent` (`freeastroapi-rust/<version> <suffix>`).
    pub fn user_agent_suffix(mut self, suffix: impl Into<String>) -> Self {
        self.user_agent = Some(suffix.into());
        self
    }

    /// Use a preconfigured `reqwest` client (proxies, custom TLS, connection pool settings).
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = Some(client);
        self
    }

    /// Build the client. Fails when no API key is set or found in `FREEASTRO_API_KEY`, or when
    /// the base URL or a header is invalid.
    pub fn build(self) -> Result<Client> {
        let api_key = match self.api_key {
            Some(key) => key,
            None => std::env::var(API_KEY_ENV).unwrap_or_default(),
        };
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err(Error::Config(format!(
                "an API key is required: call .api_key(..) or set {API_KEY_ENV}"
            )));
        }
        let mut api_key = HeaderValue::from_str(api_key)
            .map_err(|_| Error::Config("the API key contains invalid characters".to_owned()))?;
        api_key.set_sensitive(true);

        let base_url = self
            .base_url
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned())
            .trim_end_matches('/')
            .to_owned();
        let parsed = Url::parse(&base_url)
            .map_err(|e| Error::Config(format!("invalid base URL {base_url:?}: {e}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(Error::Config(format!(
                "invalid base URL {base_url:?}: expected http or https"
            )));
        }

        let user_agent = match &self.user_agent {
            Some(suffix) => format!("{USER_AGENT} {suffix}"),
            None => USER_AGENT.to_owned(),
        };
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, header_value(&user_agent)?);
        for (name, value) in &self.headers {
            headers.insert(header_name(name)?, header_value(value)?);
        }

        let http = match self.http_client {
            Some(client) => client,
            None => {
                let mut builder = reqwest::Client::builder();
                if let Some(timeout) = self.connect_timeout {
                    builder = builder.connect_timeout(timeout);
                }
                builder.build()?
            }
        };

        Ok(Client {
            inner: Arc::new(Inner {
                http,
                base_url,
                api_key,
                headers,
                timeout: self.timeout,
                max_retries: self.max_retries,
                retry_base_delay: self.retry_base_delay,
                max_retry_delay: self.max_retry_delay,
            }),
            options: RequestOptions::default(),
        })
    }
}

/// Per-request settings, applied with [`Client::with_options`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct RequestOptions {
    /// Overrides the client's per-attempt timeout.
    pub timeout: Option<Duration>,
    /// Overrides the client's retry count.
    pub max_retries: Option<u32>,
    /// Sent as `Idempotency-Key`; makes every operation safe to retry and lets the API
    /// deduplicate chat and generation requests.
    pub idempotency_key: Option<String>,
    /// Extra headers.
    pub headers: Vec<(String, String)>,
}

impl RequestOptions {
    /// Empty options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the per-attempt timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set the retry count.
    pub fn max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = Some(max_retries);
        self
    }

    /// Set the `Idempotency-Key` header.
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Add a header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    fn merge(mut self, other: RequestOptions) -> Self {
        self.timeout = other.timeout.or(self.timeout);
        self.max_retries = other.max_retries.or(self.max_retries);
        self.idempotency_key = other.idempotency_key.or(self.idempotency_key);
        self.headers.extend(other.headers);
        self
    }
}

/// A request built by the generated resource methods. Which builder methods are used depends
/// on the spec, hence the `dead_code` allowance.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct RequestSpec {
    method: Method,
    path: String,
    query: Vec<(String, String)>,
    body: Option<Bytes>,
    accept: Option<&'static str>,
    replay_safe: bool,
}

#[allow(dead_code)]
impl RequestSpec {
    pub(crate) fn new(method: Method, path: impl Into<String>) -> Self {
        RequestSpec {
            method,
            path: path.into(),
            query: Vec::new(),
            body: None,
            accept: None,
            replay_safe: true,
        }
    }

    pub(crate) fn json<B: Serialize + ?Sized>(mut self, body: &B) -> Result<Self> {
        self.body = Some(Bytes::from(
            serde_json::to_vec(body).map_err(Error::Encode)?,
        ));
        Ok(self)
    }

    /// Adds every field of a parameters struct as a query parameter. `None` fields are
    /// skipped, lists are sent as repeated parameters.
    pub(crate) fn query<P: Serialize>(mut self, params: &P) -> Result<Self> {
        let value = serde_json::to_value(params).map_err(Error::Encode)?;
        if let serde_json::Value::Object(map) = value {
            for (key, value) in map {
                match value {
                    serde_json::Value::Array(items) => {
                        for item in items {
                            if let Some(text) = query_value(item) {
                                self.query.push((key.clone(), text));
                            }
                        }
                    }
                    other => {
                        if let Some(text) = query_value(other) {
                            self.query.push((key, text));
                        }
                    }
                }
            }
        }
        Ok(self)
    }

    pub(crate) fn accept(mut self, accept: &'static str) -> Self {
        self.accept = Some(accept);
        self
    }

    pub(crate) fn not_replay_safe(mut self) -> Self {
        self.replay_safe = false;
        self
    }
}

fn query_value(value: serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s),
        other => Some(other.to_string()),
    }
}

#[allow(dead_code)] // unused when no operation has path parameters
/// Percent-encodes a path parameter (everything except RFC 3986 unreserved characters).
pub(crate) fn encode_path(value: impl fmt::Display) -> String {
    let raw = value.to_string();
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_path_segments() {
        assert_eq!(encode_path("2026-01-15"), "2026-01-15");
        assert_eq!(encode_path("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn parses_retry_after_seconds_and_dates() {
        let mut headers = HeaderMap::new();
        headers.insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
        assert_eq!(retry_after(&headers), Some(Duration::from_secs(2)));
        headers.insert(
            header::RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        assert_eq!(retry_after(&headers), Some(Duration::ZERO));
        headers.insert(header::RETRY_AFTER, HeaderValue::from_static("soon"));
        assert_eq!(retry_after(&headers), None);
    }

    #[test]
    fn jitter_is_in_range() {
        for _ in 0..100 {
            let j = jitter();
            assert!((0.0..1.0).contains(&j));
        }
    }
}
