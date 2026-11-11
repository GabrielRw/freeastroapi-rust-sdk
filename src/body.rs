use bytes::Bytes;

/// A non-JSON response body, such as a PNG or SVG chart or a PDF report.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Binary {
    /// The response `Content-Type`, e.g. `image/svg+xml` or `image/png`.
    pub content_type: Option<String>,
    /// The raw body.
    pub bytes: Bytes,
}

impl Binary {
    /// The body as UTF-8 text (for SVG and other text formats).
    pub fn text(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.bytes)
    }

    /// The media type without parameters, lowercased (`image/svg+xml`).
    pub fn media_type(&self) -> Option<String> {
        self.content_type
            .as_deref()
            .and_then(|ct| ct.split(';').next())
            .map(|ct| ct.trim().to_ascii_lowercase())
    }

    /// Consume the value and return the raw bytes.
    pub fn into_bytes(self) -> Bytes {
        self.bytes
    }
}

/// The body of an operation that can answer with JSON or with another media type
/// (for example a chart as JSON data or as an image), chosen by the response `Content-Type`.
#[derive(Debug, Clone, PartialEq)]
pub enum Content<T> {
    /// A JSON body, decoded.
    Json(T),
    /// Any other body.
    Binary(Binary),
}

impl<T> Content<T> {
    /// The decoded JSON body, if the response was JSON.
    pub fn json(&self) -> Option<&T> {
        match self {
            Content::Json(value) => Some(value),
            Content::Binary(_) => None,
        }
    }

    /// The decoded JSON body, if the response was JSON.
    pub fn into_json(self) -> Option<T> {
        match self {
            Content::Json(value) => Some(value),
            Content::Binary(_) => None,
        }
    }

    /// The raw body, if the response was not JSON.
    pub fn binary(&self) -> Option<&Binary> {
        match self {
            Content::Json(_) => None,
            Content::Binary(binary) => Some(binary),
        }
    }
}
