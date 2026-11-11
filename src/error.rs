use std::time::Duration;

/// Shorthand for `std::result::Result<T, freeastroapi::Error>`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong when calling FreeAstroAPI.
///
/// HTTP error statuses are split into the variants callers usually branch on; every one of them
/// carries the same [`ApiError`] details, available through [`Error::api_error`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The API key is missing, invalid or not allowed to call this operation (HTTP 401 or 403).
    #[error("authentication failed: {0}")]
    Authentication(Box<ApiError>),

    /// Too many requests (HTTP 429), after the configured retries were used up.
    #[error("rate limited: {error}")]
    RateLimit {
        /// The error response.
        error: Box<ApiError>,
        /// How long the server asked to wait (`Retry-After`), when it said.
        retry_after: Option<Duration>,
    },

    /// The resource does not exist (HTTP 404).
    #[error("not found: {0}")]
    NotFound(Box<ApiError>),

    /// The request was rejected as invalid (HTTP 400 or 422).
    #[error("invalid request: {0}")]
    Validation(Box<ApiError>),

    /// Any other non-success status, including server errors after the retries were used up.
    #[error("API error: {0}")]
    Api(Box<ApiError>),

    /// The request could not be sent or the response could not be read (connection, TLS,
    /// timeout). Use [`reqwest::Error::is_timeout`] to tell timeouts apart.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// The response body did not match the expected type.
    #[error("could not decode the response body: {source}")]
    Decode {
        /// The underlying JSON error.
        source: serde_json::Error,
        /// The body that failed to decode (truncated to 4 KiB).
        body: String,
    },

    /// A request value could not be serialised to JSON.
    #[error("could not encode the request: {0}")]
    Encode(serde_json::Error),

    /// The client is misconfigured (missing API key, invalid base URL or header).
    #[error("invalid configuration: {0}")]
    Config(String),
}

impl Error {
    /// The HTTP status of an error response, if this error came from one.
    pub fn status(&self) -> Option<u16> {
        self.api_error().map(|e| e.status)
    }

    /// The `x-request-id` of an error response, useful when contacting support.
    pub fn request_id(&self) -> Option<&str> {
        self.api_error().and_then(|e| e.request_id.as_deref())
    }

    /// Details of the error response, for the variants that come from one.
    pub fn api_error(&self) -> Option<&ApiError> {
        match self {
            Error::Authentication(e)
            | Error::NotFound(e)
            | Error::Validation(e)
            | Error::Api(e)
            | Error::RateLimit { error: e, .. } => Some(e),
            _ => None,
        }
    }

    /// Whether the request timed out.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Error::Http(e) if e.is_timeout())
    }

    pub(crate) fn from_response(error: ApiError, retry_after: Option<Duration>) -> Self {
        let error = Box::new(error);
        match error.status {
            401 | 403 => Error::Authentication(error),
            404 => Error::NotFound(error),
            400 | 422 => Error::Validation(error),
            429 => Error::RateLimit { error, retry_after },
            _ => Error::Api(error),
        }
    }
}

/// An error response from FreeAstroAPI.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ApiError {
    /// HTTP status code.
    pub status: u16,
    /// Human-readable message (`detail`, `message` or `error` from the body, else the status text).
    pub message: String,
    /// Machine-readable error code from the body, when present.
    pub code: Option<String>,
    /// The `x-request-id` response header (or `trace_id` from the body).
    pub request_id: Option<String>,
    /// The raw response body.
    pub body: String,
    /// The response body parsed as JSON, when it is JSON. For validation errors this holds the
    /// field errors (`detail: [{loc, msg, type}]`).
    pub details: Option<serde_json::Value>,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.status, self.message)?;
        if let Some(code) = &self.code {
            write!(f, " (code {code})")?;
        }
        if let Some(id) = &self.request_id {
            write!(f, " [request {id}]")?;
        }
        Ok(())
    }
}

impl ApiError {
    pub(crate) fn new(
        status: u16,
        reason: Option<&str>,
        request_id: Option<String>,
        body: &[u8],
    ) -> Self {
        let text = String::from_utf8_lossy(body).into_owned();
        let details: Option<serde_json::Value> = serde_json::from_slice(body).ok();
        let field = |name: &str| details.as_ref().and_then(|d| d.get(name));
        let message = [field("detail"), field("message"), field("error")]
            .into_iter()
            .flatten()
            .find_map(|value| match value {
                serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
                serde_json::Value::Array(items) => items
                    .first()
                    .and_then(|item| item.get("msg"))
                    .and_then(|msg| msg.as_str())
                    .map(|msg| msg.to_owned()),
                serde_json::Value::Object(map) => map
                    .get("message")
                    .and_then(|m| m.as_str())
                    .map(|m| m.to_owned()),
                _ => None,
            })
            .unwrap_or_else(|| reason.map_or_else(|| format!("HTTP {status}"), str::to_owned));
        let code = field("code")
            .or_else(|| field("error").and_then(|e| e.get("code")))
            .and_then(|c| c.as_str())
            .map(str::to_owned);
        let request_id = request_id.or_else(|| {
            field("trace_id")
                .or_else(|| field("request_id"))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        });
        ApiError {
            status,
            message,
            code,
            request_id,
            body: text,
            details,
        }
    }
}
