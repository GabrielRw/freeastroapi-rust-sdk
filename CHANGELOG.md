# Changelog

## 0.1.0

- First release: an async client (`reqwest` + `serde`) with a typed method for each of the 142
  operations in the public FreeAstroAPI OpenAPI document, grouped by API path
  (`client.vedic().panchang(..)`), latest version under the plain name and older ones with a
  `_v1`/`_v2` suffix.
- Request and response types for all 624 schemas (`freeastroapi::types`), query parameter
  structs (`freeastroapi::params`) and an `OPERATIONS` table.
- `Client::builder()` with `api_key` (or `FREEASTRO_API_KEY`), `base_url`, `timeout`,
  `max_retries` and more; per-request options (timeout, retries, idempotency key, headers)
  through `Client::with_options`.
- Retries with exponential backoff and jitter on 429, transient network failures and
  408/5xx, honouring `Retry-After`.
- Typed errors: `Authentication`, `RateLimit`, `NotFound`, `Validation`, `Api`, `Http`,
  `Decode`, with the status, message, code and request id of the error response.
- Binary and SVG responses as `Binary`, text responses as `String`, JSON-or-image responses as
  `Content<T>`.
- `rustls-tls` (default) and `native-tls` features.
