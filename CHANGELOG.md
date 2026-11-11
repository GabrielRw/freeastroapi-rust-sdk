# Changelog

## 0.1.0

- First release: an async client (`reqwest` + `serde`) with a typed method for each of the 142
  operations in the public FreeAstroAPI OpenAPI document, grouped by API path
  (`client.vedic().panchang(..)`), latest version under the plain name and older ones with a
  `_v1`/`_v2` suffix.
- Typed responses for every operation: request and response types for all 975 schemas
  (`freeastroapi::types`), so every JSON operation returns a Rust type instead of
  `serde_json::Value`. Query parameter structs live in `freeastroapi::params`; `OPERATIONS`
  lists every operation.
- Lenient response decoding: a missing "required" field, a `null` in a non-null field, an
  unknown field or an unknown enum value (`Other(String)`) never fails decoding. Request types
  keep their required fields. Unions pick their variant by required keys and `const`
  discriminators.
- Typed errors: `Authentication`, `RateLimit`, `NotFound`, `Validation`, `Api`, `Http`,
  `Decode`, with the status, message, code and request id of the error response (the API's
  `ErrorResponse` / `HTTPValidationError` bodies, parsed into `ApiError::details`).
- Retry policy shared with the other FreeAstroAPI SDKs: 2 retries by default, exponential
  backoff from 0.5 s (factor 2, full jitter, capped at 8 s). 429 is retried for every operation,
  honouring `Retry-After` (seconds or HTTP date) and `retry_after_ms`; a requested wait over 30 s
  is returned at once as `Error::RateLimit { retry_after, .. }`. 408/500/502/503/504, network
  errors and timeouts are retried for every operation except `SIDE_EFFECT_OPERATIONS`
  (`v1_natal_calculate`, `v1_ai_generate`, `v2_vedic_chat`, `v2_western_chat`, `v2_vedic_qa`,
  `v1_western_transits_timeline`, `v1_western_transits_insights`), which need an idempotency key
  or `RequestOptions::retry_side_effects(true)`. Other 4xx are never retried.
- `Client::builder()` with `api_key` (or `FREEASTRO_API_KEY`), `base_url`, `timeout`,
  `max_retries`, `retry_base_delay`, `max_retry_delay`, `max_retry_after` and more; per-request
  options (timeout, retries, idempotency key, side-effect retry opt-in, headers) through
  `Client::with_options`.
- Binary responses (PNG/SVG charts, PDF reports) as `Binary`, text responses as `String`,
  JSON-or-binary responses as `Content<T>`.
- `rustls-tls` (default) and `native-tls` features.
