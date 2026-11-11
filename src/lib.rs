//! Official Rust SDK for [FreeAstroAPI](https://www.freeastroapi.com): western, Vedic and
//! Chinese astrology, numerology, moon phases, horoscopes, electional search and more.
//!
//! The client, its resource methods and every request and response type are generated from the
//! API's OpenAPI document, so each of the API's operations has a typed method.
//!
//! # Quick start
//!
//! ```no_run
//! use freeastroapi::{Client, types::NatalChartRequest};
//!
//! # async fn run() -> freeastroapi::Result<()> {
//! // Reads FREEASTRO_API_KEY when no key is passed to the builder.
//! let client = Client::builder().build()?;
//!
//! let chart = client
//!     .natal()
//!     .calculate(&NatalChartRequest {
//!         year: 1990,
//!         month: 5,
//!         day: 15,
//!         hour: Some(14),
//!         minute: Some(30),
//!         city: Some("London".into()),
//!         ..Default::default()
//!     }, None)
//!     .await?;
//! println!("{chart:#?}");
//! # Ok(())
//! # }
//! ```
//!
//! # Layout
//!
//! * [`Client`] — configured with [`Client::builder`]; resource accessors such as
//!   [`Client::western`], [`Client::vedic`] and [`Client::geo`] mirror the API paths.
//!   When an operation exists in several API versions the plain method name calls the latest
//!   one and older ones carry a suffix (`vedic().chart()` is v2, `vedic().chart_v1()` is v1).
//! * [`types`] — request and response bodies, one Rust type per schema.
//! * [`params`] — query parameters, one struct per operation that takes them.
//! * [`Error`] — typed errors: [`Error::Authentication`], [`Error::RateLimit`],
//!   [`Error::NotFound`], [`Error::Validation`], [`Error::Api`], [`Error::Http`] and
//!   [`Error::Decode`].
//!
//! Failed requests are retried with exponential backoff (honouring `Retry-After`) on 429 and
//! on transient failures; see [`ClientBuilder::max_retries`].
//!
//! # Features
//!
//! * `rustls-tls` (default) — TLS through rustls.
//! * `native-tls` — TLS through the platform library. Disable default features to drop rustls.
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_debug_implementations, rust_2018_idioms, unreachable_pub)]

mod body;
mod client;
mod error;
mod generated;

pub use body::{Binary, Content};
pub use client::{
    API_KEY_ENV, Client, ClientBuilder, DEFAULT_BASE_URL, DEFAULT_MAX_RETRIES, DEFAULT_TIMEOUT,
    RequestOptions,
};
pub use error::{ApiError, Error, Result};
pub use generated::operations::OPERATIONS;
pub use generated::{params, resources, types};

/// Re-export of the HTTP client crate, for [`ClientBuilder::http_client`] and
/// [`reqwest::Method`] in [`Client::request`].
pub use reqwest;

/// One API operation and the client method that calls it. See [`OPERATIONS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct OperationInfo {
    /// The OpenAPI `operationId`, e.g. `v2_vedic_panchang`.
    pub operation_id: &'static str,
    /// HTTP method.
    pub method: &'static str,
    /// Path template, e.g. `/api/v1/chinese/calendar/{date}`.
    pub path: &'static str,
    /// Method chain on [`Client`], e.g. `["vedic", "panchang"]` for `client.vedic().panchang(..)`.
    pub accessor: &'static [&'static str],
    /// Whether the API marks the operation as deprecated.
    pub deprecated: bool,
}

/// Compiles the README's code samples as doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
