//! Smoke test against production (.github/workflows/live-smoke.yml): five cheap, read-only
//! calls with retries off, so a failure never spends extra quota. Ignored by default:
//!
//!     FREEASTRO_TEST_API_KEY=... cargo test --test live -- --ignored --test-threads 1

use std::time::Duration;

use freeastroapi::{Client, params, types};

fn client() -> Client {
    let key = std::env::var("FREEASTRO_TEST_API_KEY")
        .expect("set FREEASTRO_TEST_API_KEY to call the production API");
    let base_url = std::env::var("FREEASTRO_BASE_URL")
        .unwrap_or_else(|_| freeastroapi::DEFAULT_BASE_URL.to_owned());
    Client::builder()
        .api_key(key)
        .base_url(base_url)
        .max_retries(0)
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap()
}

fn paris_birth() -> types::NatalChartRequest {
    types::NatalChartRequest {
        year: 1990,
        month: 6,
        day: 15,
        hour: Some(14),
        minute: Some(30),
        city: Some("Paris".into()),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "calls production; needs FREEASTRO_TEST_API_KEY"]
async fn geo_search() {
    let params = params::GeoSearchParams {
        q: "Paris".into(),
        limit: Some(1),
        ..Default::default()
    };
    let cities = client().geo().search(&params).await.unwrap();
    assert!(!cities.results[0].timezone.is_empty());
}

#[tokio::test]
#[ignore = "calls production; needs FREEASTRO_TEST_API_KEY"]
async fn moon_phase() {
    let params = params::MoonPhaseParams {
        date: Some("2026-01-15".into()),
        ..Default::default()
    };
    let moon = client().moon().phase(Some(&params)).await.unwrap();
    assert!(!moon.phase.name.is_empty());
}

#[tokio::test]
#[ignore = "calls production; needs FREEASTRO_TEST_API_KEY"]
async fn western_sun_sign() {
    let sign = client()
        .western()
        .signs()
        .sun(&paris_birth())
        .await
        .unwrap();
    assert_eq!(sign.sign_id, "gemini");
}

#[tokio::test]
#[ignore = "calls production; needs FREEASTRO_TEST_API_KEY"]
async fn natal_calculate() {
    let chart = client()
        .natal()
        .calculate(&paris_birth(), None)
        .await
        .unwrap();
    assert!(chart.as_object().is_some_and(|o| !o.is_empty()));
}

#[tokio::test]
#[ignore = "calls production; needs FREEASTRO_TEST_API_KEY"]
async fn vedic_panchang() {
    let panchang = client()
        .vedic()
        .panchang(&types::PanchangRequestV2 {
            year: 2026,
            month: 1,
            day: 14,
            hour: 6,
            minute: 0,
            city: Some("Delhi".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!panchang.tithi.name.is_empty());
}
