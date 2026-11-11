# freeastroapi

[![CI](https://github.com/GabrielRw/freeastroapi-rust-sdk/actions/workflows/ci.yml/badge.svg)](https://github.com/GabrielRw/freeastroapi-rust-sdk/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/freeastroapi.svg)](https://crates.io/crates/freeastroapi)
[![docs.rs](https://img.shields.io/docsrs/freeastroapi)](https://docs.rs/freeastroapi)

Official Rust SDK for [FreeAstroAPI](https://www.freeastroapi.com): western, Vedic and Chinese
astrology, numerology, moon phases, horoscopes, electional search and more. Async, built on
`reqwest` and `serde`, with a typed method for each of the API's 142 operations and a Rust type
for every schema.

## Install

```sh
cargo add freeastroapi
cargo add tokio --features macros,rt-multi-thread
```

Requires Rust 1.85 or newer. TLS uses rustls by default; for the platform TLS library use
`cargo add freeastroapi --no-default-features --features native-tls`.

## Quick start

```rust,no_run
use freeastroapi::{Client, types::NatalChartRequest};

#[tokio::main]
async fn main() -> freeastroapi::Result<()> {
    // Reads FREEASTRO_API_KEY when no key is given.
    let client = Client::builder()
        .api_key(std::env::var("FREEASTRO_API_KEY").unwrap_or_default())
        .timeout(std::time::Duration::from_secs(30))
        .max_retries(2)
        .build()?;

    let birth = NatalChartRequest {
        year: 1990,
        month: 6,
        day: 15,
        hour: Some(14),
        minute: Some(30),
        city: Some("Paris".into()),
        ..Default::default()
    };
    let sun = client.western().signs().sun(&birth).await?;
    println!("Sun in {} at {:.2}°", sun.sign, sun.degree);
    Ok(())
}
```

`Client::new("key")` and `Client::from_env()` are shortcuts. The client is cheap to clone and
shares its connection pool.

## Examples

Runnable versions live in [`examples/`](examples) (`cargo run --example natal_chart`).

### Natal chart

```rust,no_run
use freeastroapi::{params::NatalCalculateParams, types::NatalChartRequest};

async fn run(client: &freeastroapi::Client) -> freeastroapi::Result<()> {
    let birth = NatalChartRequest {
        year: 1990, month: 6, day: 15,
        hour: Some(14), minute: Some(30),
        city: Some("Paris".into()),
        ..Default::default()
    };
    // POST /api/v1/natal/calculate?include_minor=true
    let options = NatalCalculateParams { include_minor: Some(true), ..Default::default() };
    let chart = client.natal().calculate(&birth, Some(&options)).await?;
    println!("{}", chart["planets"]);
    Ok(())
}
```

### Vedic panchang

```rust,no_run
use freeastroapi::types::PanchangRequestV2;

async fn run(client: &freeastroapi::Client) -> freeastroapi::Result<()> {
    // POST /api/v2/vedic/panchang
    let panchang = client
        .vedic()
        .panchang(&PanchangRequestV2 {
            year: 2026, month: 1, day: 14, hour: 6, minute: 0,
            city: Some("Delhi".into()),
            ..Default::default()
        })
        .await?;
    println!("{}: tithi {}, nakshatra {}", panchang.date, panchang.tithi.name, panchang.nakshatra.name);
    Ok(())
}
```

### Geo search

```rust,no_run
use freeastroapi::params::GeoSearchParams;

async fn run(client: &freeastroapi::Client) -> freeastroapi::Result<()> {
    // GET /api/v2/geo/search?q=Paris&limit=3
    let cities = client
        .geo()
        .search(&GeoSearchParams { q: "Paris".into(), limit: Some(3), ..Default::default() })
        .await?;
    for city in &cities.results {
        println!("{}, {} ({}, {}) {}", city.name, city.country, city.lat, city.lng, city.timezone);
    }
    Ok(())
}
```

## Resources

Methods follow the API paths: `client.western().progressions().secondary_exact_aspects(&body)`
calls `POST /api/v1/western/progressions/secondary/exact-aspects`. Top-level groups are `ai`,
`chinese`, `ephemeris`, `geo`, `horoscope`, `moon`, `natal`, `numerology`, `sky_events`, `space`,
`transits`, `vedic` and `western` (with `astrocartography`, `chart_similarity`, `directions`,
`electional`, `natal`, `profections`, `progressions`, `returns`, `signs`, `solar` and
`transits`).

When an operation exists in several API versions the plain name calls the latest one and older
versions carry a suffix: `client.vedic().chart(..)` calls `/api/v2/vedic/chart`,
`client.vedic().chart_v1(..)` calls `/api/v1/vedic/chart`. Rust keywords are raw identifiers
(`client.vedic().r#match(..)`). `freeastroapi::OPERATIONS` lists every operation with its
method chain.

Arguments come in a fixed order: path parameters (anything `Display`), then the JSON body by
reference, then query parameters as a `params::*Params` struct (`Option<&_>` when every
parameter is optional).

### Types

- `freeastroapi::types` has one type per schema. Every type implements `Default`, `Clone`,
  `Debug`, `PartialEq`, `Serialize` and `Deserialize`; optional fields are `Option`s that are
  left out of the request so the server applies its documented defaults (shown in the field docs).
- String enums have an `Other(String)` variant, so values added to the API later still decode.
- Operations whose response the spec does not describe yet return `serde_json::Value`; they
  become typed as the API's spec gains response schemas.
- Image responses (`client.vedic().kp_render(..)`, `client.vedic().visual().chart(..)`) return
  `Binary { content_type, bytes }`; use `.text()` for SVG. Text-only responses return `String`,
  and operations that can answer with JSON or an image return `Content<T>`.
- `client.request::<T>(method, path, query, body)` calls any endpoint, including ones added to
  the API after your SDK version.

## Errors, retries and timeouts

Every method returns `freeastroapi::Result<T>`. `Error` distinguishes `Authentication` (401/403),
`RateLimit { retry_after, .. }` (429), `NotFound` (404), `Validation` (400/422), `Api` (other
statuses), `Http` (connection, TLS, timeout) and `Decode`. API errors carry an `ApiError` with
the status, message, error code, `x-request-id`, raw body and parsed JSON details:

```rust,no_run
async fn run(client: &freeastroapi::Client, params: freeastroapi::params::GeoSearchParams) {
    match client.geo().search(&params).await {
        Ok(cities) => println!("{} results", cities.count),
        Err(freeastroapi::Error::RateLimit { retry_after, .. }) => println!("retry after {retry_after:?}"),
        Err(error) => eprintln!("{error} (request id {:?})", error.request_id()),
    }
}
```

Requests time out after 30 s per attempt (`ClientBuilder::timeout`). Failed requests are retried
up to `max_retries` times (default 2) with exponential backoff and jitter, honouring
`Retry-After`:

- 429 is always retried.
- Connection errors, timeouts and 408/500/502/503/504 are retried for operations that are safe to
  replay: all the read-only calculations (GET and POST). Chat and AI generation are only retried
  on these when you set an idempotency key.

Per-request settings go through a derived client that shares the connection pool:

```rust,no_run
async fn run(client: &freeastroapi::Client, body: freeastroapi::types::VedicChatRequest) -> freeastroapi::Result<()> {
    use freeastroapi::RequestOptions;

    let reply = client
        .with_options(RequestOptions::new().idempotency_key("chat-42").timeout(std::time::Duration::from_secs(120)))
        .vedic()
        .chat(&body)
        .await?;
    let _ = reply;
    Ok(())
}
```

Each request sends `x-api-key` and `User-Agent: freeastroapi-rust/<version>`.

## Development

The SDK is generated from the vendored OpenAPI document `spec/freeastro.json` by
`scripts/generate.py` (Python 3.9+, standard library only, formats its output with rustfmt).
Generated code is committed, so building the crate never runs the generator.

```sh
python3 scripts/spec_sync.py fetch          # update spec/freeastro.json from production
python3 scripts/generate.py                 # regenerate src/generated/ and tests/{operations,contract}.rs
python3 scripts/generate.py --check         # CI: generated code is current
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo test                                  # unit, transport, coverage and per-operation tests
python3 scripts/check_fixtures.py           # generator against synthetic specs (SVG/PNG/PDF/markdown, errors, 3.1)
python3 scripts/with_prism.py -- cargo test --test contract -- --ignored     # every method against a Prism mock
FREEASTRO_TEST_API_KEY=... cargo test --test live -- --ignored --test-threads 1  # production smoke test
```

`scripts/spec_sync.py`, `scripts/with_prism.py` and `tests/fixtures/synthetic-spec.json` are
shared with the other FreeAstroAPI SDKs. The `Sync spec` workflow fetches the production spec
daily and opens a pull request from `spec-sync/auto` when it changed.

## License

Apache-2.0. See [LICENSE](LICENSE).
