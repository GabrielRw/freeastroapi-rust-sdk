//! Transport behaviour: authentication, retries, error mapping, request and response encoding.

mod common;

use std::time::Duration;

use common::{API_KEY, client_for, json_response, raw_response, setup};
use freeastroapi::{Client, Error, RequestOptions, params, types};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CITIES: &str = r#"{"results": [{"name": "Paris", "country": "FR", "lat": 48.85, "lng": 2.35, "timezone": "Europe/Paris", "population": 2133111}], "count": 1}"#;

fn paris() -> params::GeoSearchParams {
    params::GeoSearchParams {
        q: "Paris".into(),
        limit: Some(3),
        ..Default::default()
    }
}

#[tokio::test]
async fn sends_api_key_user_agent_and_accept_headers() {
    let (server, client) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/geo/search"))
        .and(header("x-api-key", API_KEY))
        .and(header("accept", "application/json"))
        .and(header(
            "user-agent",
            concat!("freeastroapi-rust/", env!("CARGO_PKG_VERSION")),
        ))
        .respond_with(json_response(200, CITIES))
        .expect(1)
        .mount(&server)
        .await;
    client.geo().search(&paris()).await.unwrap();
}

#[tokio::test]
async fn query_call_encodes_params_and_decodes_typed_response() {
    let (server, client) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/geo/search"))
        .and(query_param("q", "Paris"))
        .and(query_param("limit", "3"))
        .respond_with(json_response(200, CITIES))
        .expect(1)
        .mount(&server)
        .await;
    let cities = client.geo().search(&paris()).await.unwrap();
    assert_eq!(cities.count, 1);
    assert_eq!(cities.results[0].name, "Paris");
    let request = &server.received_requests().await.unwrap()[0];
    // `None` fields are not sent; parameters go out in name order.
    assert_eq!(request.url.query(), Some("limit=3&q=Paris"));
}

#[tokio::test]
async fn list_query_params_are_repeated() {
    let (server, client) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/western/profections/annual"))
        .respond_with(json_response(200, "{}"))
        .mount(&server)
        .await;
    let params = params::WesternProfectionsAnnualParams {
        include: Some(vec!["monthly".into(), "natal_reference".into()]),
    };
    let body = types::AnnualProfectionRequest::default();
    let _ = client
        .western()
        .profections()
        .annual(&body, Some(&params))
        .await;
    let request = &server.received_requests().await.unwrap()[0];
    assert_eq!(
        request.url.query(),
        Some("include=monthly&include=natal_reference")
    );
}

#[tokio::test]
async fn body_call_sends_json_and_omits_unset_fields() {
    let (server, client) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/vedic/panchang"))
        .and(header("content-type", "application/json"))
        .and(body_json(json!({
            "year": 2026, "month": 1, "day": 14, "hour": 6, "minute": 0, "city": "Delhi"
        })))
        .respond_with(json_response(200, r#"{"date": 20260114}"#))
        .expect(1)
        .mount(&server)
        .await;
    let body = types::PanchangRequestV2 {
        year: 2026,
        month: 1,
        day: 14,
        hour: 6,
        minute: 0,
        city: Some("Delhi".into()),
        ..Default::default()
    };
    // Decoding is lenient about missing fields, but a number where a string belongs still fails
    // after the request matched.
    let error = client.vedic().panchang(&body).await.unwrap_err();
    assert!(matches!(error, Error::Decode { .. }), "{error:?}");
}

#[tokio::test]
async fn path_params_are_percent_encoded() {
    let (server, client) = setup().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/natal/report/job%2F1%20a"))
        .respond_with(json_response(200, "{}"))
        .expect(1)
        .mount(&server)
        .await;
    client.natal().report("job/1 a", None).await.unwrap();
}

#[tokio::test]
async fn svg_and_png_responses_are_returned_raw() {
    let (server, client) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/vedic/kp/render"))
        .respond_with(raw_response(200, "image/svg+xml", b"<svg/>"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/vedic/visual/chart"))
        .respond_with(raw_response(200, "image/png", b"\x89PNG\r\n\x1a\n"))
        .mount(&server)
        .await;

    let svg = client
        .vedic()
        .kp_render(&types::KPChartRenderRequest::default())
        .await
        .unwrap();
    assert_eq!(svg.media_type().as_deref(), Some("image/svg+xml"));
    assert_eq!(svg.text().unwrap(), "<svg/>");
    let request = &server.received_requests().await.unwrap()[0];
    assert_eq!(
        request.headers.get("accept").unwrap(),
        "image/png, image/svg+xml, application/json"
    );

    let png = client
        .vedic()
        .visual()
        .chart(&types::VedicVisualChartRequest::default())
        .await
        .unwrap();
    assert_eq!(png.content_type.as_deref(), Some("image/png"));
    assert_eq!(&png.bytes[..4], b"\x89PNG");
}

#[tokio::test]
async fn maps_error_statuses() {
    let (server, client) = setup().await;
    let cases: [(u16, &str); 5] = [
        (401, r#"{"detail": "Invalid API key"}"#),
        (404, r#"{"detail": "Not Found"}"#),
        (
            422,
            r#"{"detail": [{"loc": ["body", "year"], "msg": "Field required", "type": "missing"}]}"#,
        ),
        (429, r#"{"detail": "Too many requests"}"#),
        (500, r#"{"error": {"code": "internal", "message": "boom"}}"#),
    ];
    for (status, body) in cases {
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/geo/search"))
            .respond_with(json_response(status, body).insert_header("x-request-id", "req-123"))
            .mount(&server)
            .await;
        let error = client.geo().search(&paris()).await.unwrap_err();
        assert_eq!(error.status(), Some(status));
        assert_eq!(error.request_id(), Some("req-123"));
        let api = error.api_error().unwrap();
        assert_eq!(api.body, body);
        match status {
            401 => {
                assert!(matches!(error, Error::Authentication(_)));
                assert_eq!(api.message, "Invalid API key");
            }
            404 => assert!(matches!(error, Error::NotFound(_))),
            422 => {
                assert!(matches!(error, Error::Validation(_)));
                assert_eq!(api.message, "Field required");
                assert!(api.details.is_some());
            }
            429 => assert!(matches!(error, Error::RateLimit { .. })),
            _ => {
                assert!(matches!(error, Error::Api(_)));
                assert_eq!(api.message, "boom");
                assert_eq!(api.code.as_deref(), Some("internal"));
            }
        }
    }
}

#[tokio::test]
async fn non_json_error_bodies_keep_the_status_text() {
    let (server, client) = setup().await;
    Mock::given(method("GET"))
        .respond_with(raw_response(502, "text/html", b"<html>bad gateway</html>"))
        .mount(&server)
        .await;
    let error = client.geo().search(&paris()).await.unwrap_err();
    let api = error.api_error().unwrap();
    assert_eq!(api.status, 502);
    assert_eq!(api.message, "Bad Gateway");
    assert_eq!(api.details, None);
}

#[tokio::test]
async fn retries_429_honouring_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/western/signs/sun"))
        .respond_with(
            json_response(429, r#"{"detail": "slow down"}"#).insert_header("retry-after", "0"),
        )
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/western/signs/sun"))
        .respond_with(json_response(
            200,
            r#"{"point": "Sun", "sign": "Gemini", "sign_id": "gemini", "degree": 24.1, "abs_pos": 84.1, "house": 10}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server, 2);
    let sign = client
        .western()
        .signs()
        .sun(&types::NatalChartRequest::default())
        .await
        .unwrap();
    assert_eq!(sign.sign_id, types::SignId::Gemini);
}

#[tokio::test]
async fn gives_up_after_max_retries_with_rate_limit_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(json_response(429, "{}").insert_header("retry-after", "0"))
        .expect(3)
        .mount(&server)
        .await;
    let error = client_for(&server, 2)
        .geo()
        .search(&paris())
        .await
        .unwrap_err();
    match error {
        Error::RateLimit { retry_after, .. } => assert_eq!(retry_after, Some(Duration::ZERO)),
        other => panic!("expected RateLimit, got {other:?}"),
    }
}

#[tokio::test]
async fn does_not_wait_for_a_retry_after_beyond_the_cap() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(json_response(429, "{}").insert_header("retry-after", "3600"))
        .expect(1)
        .mount(&server)
        .await;
    let error = client_for(&server, 2)
        .geo()
        .search(&paris())
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::RateLimit { retry_after: Some(d), .. } if d == Duration::from_secs(3600))
    );
}

#[tokio::test]
async fn does_not_wait_for_a_retry_after_ms_beyond_thirty_seconds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(json_response(
            429,
            r#"{"error": "rate_limited", "retry_after_ms": 31000}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;
    let error = client_for(&server, 2)
        .ephemeris()
        .calculate(&types::EphemerisRequest::default())
        .await
        .unwrap_err();
    match error {
        Error::RateLimit { retry_after, .. } => {
            assert_eq!(retry_after, Some(Duration::from_secs(31)))
        }
        other => panic!("expected RateLimit, got {other:?}"),
    }
}

#[tokio::test]
async fn retries_429_on_a_side_effect_post_and_honours_retry_after_ms() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/vedic/chat"))
        .respond_with(json_response(
            429,
            r#"{"error": "rate_limited", "retry_after_ms": 1}"#,
        ))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/vedic/chat"))
        .respond_with(json_response(200, "{}"))
        .expect(1)
        .mount(&server)
        .await;
    client_for(&server, 2)
        .vedic()
        .chat(&types::VedicChatRequest::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn retries_503_on_a_calculation_post() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/ephemeris/calculate"))
        .respond_with(json_response(503, "{}"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/ephemeris/calculate"))
        .respond_with(json_response(200, "{}"))
        .expect(1)
        .mount(&server)
        .await;
    client_for(&server, 1)
        .ephemeris()
        .calculate(&types::EphemerisRequest::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn does_not_retry_other_client_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(json_response(400, r#"{"detail": "bad"}"#))
        .expect(1)
        .mount(&server)
        .await;
    let error = client_for(&server, 3)
        .ephemeris()
        .calculate(&types::EphemerisRequest::default())
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Validation(_)));
}

#[tokio::test]
async fn does_not_retry_a_server_error_asking_for_a_long_wait() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(json_response(503, "{}").insert_header("retry-after", "120"))
        .expect(1)
        .mount(&server)
        .await;
    let error = client_for(&server, 3)
        .ephemeris()
        .calculate(&types::EphemerisRequest::default())
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(503));
}

#[test]
fn side_effect_operations_match_the_shared_list() {
    assert_eq!(
        freeastroapi::SIDE_EFFECT_OPERATIONS,
        [
            "v1_natal_calculate",
            "v1_ai_generate",
            "v2_vedic_chat",
            "v2_western_chat",
            "v2_vedic_qa",
            "v1_western_transits_timeline",
            "v1_western_transits_insights",
        ]
    );
    for id in freeastroapi::SIDE_EFFECT_OPERATIONS {
        assert!(
            freeastroapi::OPERATIONS
                .iter()
                .any(|op| op.operation_id == *id),
            "{id} is not in the spec"
        );
    }
}

#[tokio::test]
async fn side_effect_operations_are_not_retried_on_503_without_a_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(json_response(503, "{}"))
        .expect(3)
        .mount(&server)
        .await;
    let client = client_for(&server, 3);
    let error = client
        .vedic()
        .chat(&types::VedicChatRequest::default())
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(503));
    let error = client
        .natal()
        .calculate(&types::NatalChartRequest::default(), None)
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(503));
    // The raw escape hatch applies the same rule to a known path.
    let error = client
        .request::<serde_json::Value>(
            freeastroapi::reqwest::Method::POST,
            "/api/v2/vedic/chat",
            &[],
            Some(&json!({})),
        )
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(503));
}

#[tokio::test]
async fn idempotency_key_is_sent_and_enables_retries_on_503() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/vedic/chat"))
        .and(header("idempotency-key", "chat-1"))
        .and(header("x-trace", "abc"))
        .respond_with(json_response(503, "{}"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/vedic/chat"))
        .and(header("idempotency-key", "chat-1"))
        .respond_with(json_response(200, "{}"))
        .expect(1)
        .mount(&server)
        .await;
    let client = client_for(&server, 1).with_options(
        RequestOptions::new()
            .idempotency_key("chat-1")
            .header("x-trace", "abc"),
    );
    client
        .vedic()
        .chat(&types::VedicChatRequest::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn opting_in_retries_side_effect_operations_without_a_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/vedic/chat"))
        .respond_with(json_response(503, "{}"))
        .expect(3)
        .mount(&server)
        .await;
    let error = client_for(&server, 2)
        .with_options(RequestOptions::new().retry_side_effects(true))
        .vedic()
        .chat(&types::VedicChatRequest::default())
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Api(_)));
    let request = &server.received_requests().await.unwrap()[0];
    assert!(!request.headers.contains_key("idempotency-key"));
}

#[tokio::test]
async fn lenient_decoding_tolerates_schema_drift() {
    let (server, client) = setup().await;
    // `count` (required) is missing, `name` (required, non-null) is null, there is an unknown
    // field, and `phase.name` carries a value the enum does not know.
    Mock::given(method("GET"))
        .and(path("/api/v2/geo/search"))
        .respond_with(json_response(
            200,
            r#"{"results": [{"name": null, "country": "FR", "lat": 48.85, "lng": 2.35, "timezone": "Europe/Paris", "brand_new": {"x": 1}}], "added_later": true}"#,
        ))
        .mount(&server)
        .await;
    let cities = client.geo().search(&paris()).await.unwrap();
    assert_eq!(cities.count, 0);
    assert_eq!(cities.results[0].name, "");
    assert_eq!(cities.results[0].country, "FR");

    let moon: types::MoonPhaseResponse = serde_json::from_value(json!({
        "phase": {"name": "Super Blue Blood Moon", "illumination": null},
        "unknown": [1, 2, 3]
    }))
    .unwrap();
    assert_eq!(
        moon.phase.name,
        types::Name::Other("Super Blue Blood Moon".into())
    );
}

#[tokio::test]
async fn timeouts_surface_as_http_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(500)))
        .mount(&server)
        .await;
    let client = client_for(&server, 0)
        .with_options(RequestOptions::new().timeout(Duration::from_millis(50)));
    let error = client.geo().search(&paris()).await.unwrap_err();
    assert!(error.is_timeout(), "{error:?}");
}

#[tokio::test]
async fn generic_request_escape_hatch() {
    let (server, client) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/v9/new/thing"))
        .and(query_param("a", "1"))
        .and(body_json(json!({"x": 1})))
        .respond_with(json_response(200, r#"{"ok": true}"#))
        .expect(1)
        .mount(&server)
        .await;
    let value: serde_json::Value = client
        .request(
            freeastroapi::reqwest::Method::POST,
            "/api/v9/new/thing",
            &[("a", "1")],
            Some(&json!({"x": 1})),
        )
        .await
        .unwrap();
    assert_eq!(value["ok"], true);
}

#[test]
fn builder_validates_configuration() {
    let missing = Client::builder().api_key("  ").build().unwrap_err();
    assert!(matches!(missing, Error::Config(_)));
    let bad_url = Client::builder()
        .api_key("k")
        .base_url("ftp://example.com")
        .build()
        .unwrap_err();
    assert!(matches!(bad_url, Error::Config(_)));
    let client = Client::builder()
        .api_key("secret-key")
        .base_url("https://example.com/")
        .build()
        .unwrap();
    assert_eq!(client.base_url(), "https://example.com");
    assert!(!format!("{client:?}").contains("secret-key"));
}

#[test]
fn operations_table_lists_unique_accessors() {
    let mut seen = std::collections::HashSet::new();
    for op in freeastroapi::OPERATIONS {
        assert!(
            seen.insert(op.accessor.join(".")),
            "duplicate {}",
            op.operation_id
        );
    }
}

#[test]
fn lenient_unions_still_pick_the_matching_variant() {
    // Required keys decide between two lenient object variants.
    let table: types::WesternProgressionsQuaternaryResponse = serde_json::from_value(json!({
        "meta": {}, "input": {}, "progression_basis": {}, "tables": {}
    }))
    .unwrap();
    assert!(matches!(
        table,
        types::WesternProgressionsQuaternaryResponse::ProgressionTableResponse(_)
    ));
    let unavailable: types::NatalCalculateResponseInterpretation =
        serde_json::from_value(json!({"error": "interpretation_unavailable"})).unwrap();
    assert!(matches!(
        unavailable,
        types::NatalCalculateResponseInterpretation::InterpretationUnavailable(_)
    ));
    // `const` discriminators decide even when required fields are missing.
    let kabbalah: types::NumerologyProfileResponseData =
        serde_json::from_value(json!({"kind": "kabbalah"})).unwrap();
    assert!(matches!(
        kabbalah,
        types::NumerologyProfileResponseData::KabbalahData(_)
    ));
    // An unknown discriminator still decodes (into the first variant, with `Other`).
    let future: types::NumerologyProfileResponseData =
        serde_json::from_value(json!({"kind": "vedic_new", "extra": 1})).unwrap();
    assert!(matches!(
        future,
        types::NumerologyProfileResponseData::PythagoreanData(_)
    ));
}
